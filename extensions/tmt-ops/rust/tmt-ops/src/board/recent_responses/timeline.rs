//! Admission of Core's committed conversation page; order keys and cursors stay opaque.
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Body {
    pub text: String,
    pub truncated: bool,
    pub original_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::board) struct Sender {
    pub identity: Option<String>,
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Input {
    pub request: String,
    pub sender: Sender,
    pub uncertain: bool,
    pub body: Option<Body>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) enum Kind {
    Input(Input),
    Final {
        response_id: String,
        available: bool,
    },
    Reply {
        request: String,
        body: Option<Body>,
    },
    Failure {
        code: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Entry {
    pub id: String,
    pub batch: Option<String>,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Page {
    pub entries: Vec<Entry>,
    pub responses: HashMap<String, super::Item>,
    pub coverage: super::Coverage,
    pub capture: Option<bool>,
    pub next_before: Option<String>,
}

fn string(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn optional(value: &Value, field: &str) -> Option<Option<String>> {
    value.get(field).map_or(Some(None), |v| string(v).map(Some))
}

fn body(value: &Value) -> Option<Option<Body>> {
    match value["availability"].as_str()? {
        "unavailable" => (!["text", "truncated", "originalBytes"]
            .iter()
            .any(|field| value.get(field).is_some()))
        .then_some(None),
        "retained" => {
            let text = value["text"].as_str()?;
            let original_bytes = value["originalBytes"].as_u64()?;
            if text.len() > super::MAX_TEXT_BYTES || original_bytes < text.len() as u64 {
                return None;
            }
            Some(Some(Body {
                text: text.into(),
                truncated: value["truncated"].as_bool()?,
                original_bytes,
            }))
        }
        _ => None,
    }
}

pub(super) fn parse(identity: &str, value: &Value) -> Option<Page> {
    let values = value["entries"].as_array()?;
    if values.len() > super::MAX_ITEMS {
        return None;
    }
    let mut canonical = value.clone();
    canonical["items"] = value["responses"].clone();
    canonical.as_object_mut()?.remove("nextBefore");
    let finals = super::parse(identity, &canonical)?;
    let mut responses = HashMap::new();
    for item in finals.items {
        if responses.insert(item.response_id.clone()?, item).is_some() {
            return None;
        }
    }
    let mut entries = Vec::with_capacity(values.len());
    let mut ids = std::collections::HashSet::new();
    for value in values {
        let id = string(&value["entryId"])?;
        if !ids.insert(id.clone()) {
            return None;
        }
        let batch = optional(value, "inputBatchId")?;
        let kind = match value["kind"].as_str()? {
            "input" => {
                let sender = &value["sender"];
                sender.as_object()?;
                Kind::Input(Input {
                    request: string(&value["requestId"])?,
                    sender: Sender {
                        identity: optional(sender, "identityId")?,
                        name: optional(sender, "name")?,
                    },
                    uncertain: match value["delivery"].as_str()? {
                        "delivered" => false,
                        "uncertain" => true,
                        _ => return None,
                    },
                    body: body(value)?,
                })
            }
            "final" => {
                let response_id = string(&value["responseId"])?;
                let available = match value["availability"].as_str()? {
                    "retained" => true,
                    "unavailable" => false,
                    _ => return None,
                };
                if available && !responses.contains_key(&response_id) {
                    return None;
                }
                Kind::Final {
                    response_id,
                    available,
                }
            }
            "reply" => Kind::Reply {
                request: string(&value["requestId"])?,
                body: body(value)?,
            },
            "failure" => {
                let code = value["failure"]["code"].as_str()?;
                if !matches!(code, "worker_error" | "budget_timeout" | "storage_error") {
                    return None;
                }
                Kind::Failure { code: code.into() }
            }
            _ => return None,
        };
        entries.push(Entry { id, batch, kind });
    }
    Some(Page {
        entries,
        responses,
        coverage: finals.coverage,
        capture: finals.capture,
        next_before: optional(value, "nextBefore")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const ID: &str = "11111111-1111-4111-8111-111111111111";

    fn page() -> Value {
        json!({"identityId":ID,"entries":[
            {"entryId":"opaque-z","inputBatchId":"batch-g","kind":"input","requestId":"person",
             "sender":{"identityId":ID,"name":"Ben"},"delivery":"delivered","availability":"retained",
             "text":"Ben question","truncated":false,"originalBytes":12},
            {"entryId":"opaque-a","inputBatchId":"batch-g","kind":"final","responseId":"rsp_A","availability":"retained"},
            {"entryId":"opaque-y","inputBatchId":"batch-g","kind":"input","requestId":"digest",
             "sender":{"name":"core-1"},"delivery":"delivered","availability":"retained",
             "text":"Digest question","truncated":false,"originalBytes":15},
            {"entryId":"opaque-b","inputBatchId":"batch-g","kind":"final","responseId":"rsp_B","availability":"retained"}
        ],"responses":[
            {"responseId":"rsp_A","text":"Answer A","provider":"claude","endedMs":2000,"truncated":false,"originalBytes":8},
            {"responseId":"rsp_B","text":"Answer B","provider":"claude","endedMs":1000,"truncated":false,"originalBytes":8}
        ],"coverage":{"olderDropped":false},"capture":{"enabled":true},"nextBefore":"opaque-page"})
    }

    #[test]
    fn incident_keeps_committed_entries_despite_opaque_keys_and_clock_rollback() {
        let held = parse(ID, &page()).expect("the documented timeline page");
        assert_eq!(
            held.entries
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            ["opaque-z", "opaque-a", "opaque-y", "opaque-b"]
        );
        assert_eq!(held.next_before.as_deref(), Some("opaque-page"));
        assert_eq!(held.responses["rsp_A"].text, "Answer A");
        assert_eq!(held.responses["rsp_B"].text, "Answer B");
    }

    #[test]
    fn unavailable_input_unknown_sender_and_uncertain_handoff_are_explicit() {
        let mut value = page();
        value["entries"][0]["sender"] = json!({});
        value["entries"][0]["delivery"] = json!("uncertain");
        value["entries"][0]["availability"] = json!("unavailable");
        for field in ["text", "truncated", "originalBytes"] {
            value["entries"][0].as_object_mut().unwrap().remove(field);
        }
        let held = parse(ID, &value).unwrap();
        let Kind::Input(input) = &held.entries[0].kind else {
            panic!("input")
        };
        assert!(input.sender.name.is_none() && input.sender.identity.is_none());
        assert!(input.uncertain && input.body.is_none());
    }

    #[test]
    fn refused_pages_never_become_empty_success() {
        for corrupt in 0..6 {
            let mut value = page();
            match corrupt {
                0 => value["identityId"] = json!("wrong"),
                1 => value["nextBefore"] = Value::Null,
                2 => value["entries"][0]["text"] = json!("x".repeat(16_385)),
                3 => value["entries"][1]["responseId"] = json!("missing"),
                4 => value["entries"][0]["delivery"] = json!("held"),
                _ => value["entries"] = json!(vec![value["entries"][0].clone(); 51]),
            }
            assert!(parse(ID, &value).is_none(), "corruption {corrupt}");
        }
    }
}
