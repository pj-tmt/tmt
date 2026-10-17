//! One collection request on the existing private serve socket. A doubtful mutation is unknown;
//! no retry, offline fallback or request ledger is introduced for display-only owner data.
use super::{Command, Fault, Reply};
use crate::{Result, keyring::Layout, limits};
use serde::{Deserialize, Serialize};

pub const LOCAL_PATH: &str = "/.tmt/colab/local/collections";
pub const PATH: &str = "/api/collections";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u8,
    command: Command,
}

pub fn read_only(command: &Command) -> bool {
    matches!(command, Command::List {} | Command::Show { .. })
}

pub(crate) fn parse(body: &[u8], person: bool) -> Result<Command> {
    let request: Request = serde_json::from_slice(body).map_err(|_| Fault::Invalid)?;
    if request.version != 1 {
        return Err(Fault::ServerMismatch.into());
    }
    if !request.command.valid(person) {
        return Err(Fault::Invalid.into());
    }
    Ok(request.command)
}

pub fn request(layout: &Layout, command: Command) -> Result<Reply> {
    if !command.valid(false) {
        return Err(Fault::Invalid.into());
    }
    let read = read_only(&command);
    let doubtful = || {
        if read {
            Fault::Unavailable
        } else {
            Fault::Unknown
        }
    };
    let body = serde_json::to_vec(&Request {
        version: 1,
        command: command.clone(),
    })?;
    let socket = crate::ipc::send(layout, LOCAL_PATH, &body).map_err(|_| doubtful())?;
    let reply = crate::ipc::receive_up_to(
        socket,
        limits::ACQUISITION + limits::RESPONSE,
        limits::COLLECTION_REPLY_BYTES,
    )
    .map_err(|_| doubtful())?;
    if reply.code == 200 {
        let reply: Reply = serde_json::from_slice(&reply.body).map_err(|_| doubtful())?;
        return if reply.valid() && reply.matches(&command) {
            Ok(reply)
        } else {
            Err(doubtful().into())
        };
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Refusal {
        code: String,
        message: String,
    }
    let error: Refusal = serde_json::from_slice(&reply.body).map_err(|_| doubtful())?;
    // The message is presentation only; only the closed code chooses a known refusal.
    let _ = error.message;
    Err(match error.code.as_str() {
        "COLAB_INPUT_INVALID" => Fault::Invalid,
        "COLAB_CAPACITY" => Fault::Capacity,
        "COLAB_CONFLICT" => Fault::Conflict,
        "COLAB_COLLECTION_NOT_FOUND" => Fault::NotFound,
        "COLAB_SERVER_MISMATCH" => Fault::ServerMismatch,
        _ => doubtful(),
    }
    .into())
}

/// Closed errors only: never send a database path, request text or unexpected exception to a tab.
pub(crate) fn error(error: &(dyn std::error::Error + 'static)) -> (u16, Vec<u8>) {
    let (status, code, message) = if let Some(fault) = error.downcast_ref::<Fault>() {
        let status = match fault {
            Fault::Invalid => 400,
            Fault::NotFound => 404,
            Fault::Conflict => 409,
            _ => 503,
        };
        (status, fault.code(), fault.to_string())
    } else if let Some(code) = error.downcast_ref::<crate::registration::Code>() {
        (code.status(), code.text(), code.text().to_owned())
    } else {
        (
            503,
            Fault::Unavailable.code(),
            Fault::Unavailable.to_string(),
        )
    };
    (
        status,
        serde_json::to_vec(&serde_json::json!({"code":code,"message":message}))
            .expect("fixed collection error"),
    )
}

pub(crate) fn encode(reply: &Reply) -> Result<Vec<u8>> {
    if !reply.valid() {
        return Err(Fault::Unavailable.into());
    }
    let bytes = serde_json::to_vec(reply)?;
    if bytes.len() > limits::COLLECTION_REPLY_BYTES {
        return Err(Fault::Capacity.into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_local_protocol_refuses_person_operations_unknown_fields_and_other_versions() {
        let id = "10000000-0000-4000-8000-000000000001";
        let body = |command| serde_json::to_vec(&json!({"version":1,"command":command})).unwrap();
        assert!(parse(&body(json!({"op":"list"})), false).is_ok());
        for command in [
            json!({"op":"seen","id":id,"target":id}),
            json!({"op":"clear-seen","id":id}),
        ] {
            let bytes = body(command);
            assert!(parse(&bytes, false).is_err());
            assert!(parse(&bytes, true).is_ok());
        }
        for value in [
            json!({"version":2,"command":{"op":"list"}}),
            json!({"version":1,"command":{"op":"list","page":"x"}}),
            json!({"version":1,"command":{"op":"list"},"person":true}),
        ] {
            assert!(
                parse(&serde_json::to_vec(&value).unwrap(), false).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn a_query_without_optional_needs_you_round_trips_on_the_private_socket() {
        let command = Command::Query {
            id: "10000000-0000-4000-8000-000000000001".into(),
            search: super::super::Search::default(),
            needs_you: None,
        };
        let bytes = serde_json::to_vec(&Request {
            version: 1,
            command,
        })
        .unwrap();
        let parsed = parse(&bytes, false).unwrap();
        assert!(matches!(
            parsed,
            Command::Query {
                needs_you: None,
                ..
            }
        ));
        assert!(!String::from_utf8(bytes).unwrap().contains("needsYou"));
    }

    #[test]
    fn unexpected_failures_do_not_expose_request_or_database_details() {
        let (status, body) = error(&std::io::Error::other("private/path?secret=payload"));
        assert_eq!(status, 503);
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["code"], "COLAB_UNAVAILABLE");
        assert!(!String::from_utf8(body).unwrap().contains("private/path"));
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    use crate::collections::{Collection, Definition, Detail, Item, Sort};

    fn id(n: usize) -> String {
        format!("10000000-0000-4000-8000-{n:012}")
    }
    fn collection() -> Collection {
        Collection {
            id: id(1),
            name: "🚀".repeat(64),
            maintainer: "\"".repeat(128),
            archived: false,
            definition: Definition::Manual { sort: Sort::Manual },
        }
    }

    #[test]
    fn a_full_reference_detail_and_directory_fit_the_reply_bound_with_json_escaping() {
        let value = collection();
        let items = (0..crate::collections::ITEMS)
            .map(|n| Item {
                id: id(100 + n),
                page_id: id(2),
                anchor: Some("x".repeat(128)),
                note: "\"".repeat(512),
                audience: "\"".repeat(128),
            })
            .collect();
        let detail = Reply::Detail {
            detail: Detail {
                collection: value.clone(),
                items,
                seen: vec![],
            },
            changed: false,
        };
        assert!(detail.valid());
        assert!(encode(&detail).unwrap().len() < limits::COLLECTION_REPLY_BYTES);
        let values = (0..crate::collections::COLLECTIONS)
            .map(|n| Collection {
                id: id(100 + n),
                ..value.clone()
            })
            .collect();
        assert!(
            encode(&Reply::Directory {
                collections: values
            })
            .unwrap()
            .len()
                < limits::COLLECTION_REPLY_BYTES
        );
        let smart = Collection {
            definition: Definition::Smart {
                search: crate::collections::Search {
                    q: Some("字".repeat(256)),
                    label: (0..8).map(|n| format!("{}{n}", "🚀".repeat(31))).collect(),
                    by: (0..8).map(|n| format!("{}{n}", "\"".repeat(127))).collect(),
                    sort: Some(Sort::Title),
                    archived: Some(true),
                },
                needs_you: Some(true),
            },
            ..value
        };
        let values = (0..crate::collections::COLLECTIONS)
            .map(|n| Collection {
                id: id(100 + n),
                ..smart.clone()
            })
            .collect();
        assert!(
            encode(&Reply::Directory {
                collections: values
            })
            .unwrap()
            .len()
                < limits::COLLECTION_REPLY_BYTES
        );
        let seen = (0..limits::PAGES)
            .map(|n| crate::collections::Seen {
                id: id(100 + n),
                at_ms: 9007199254740991,
            })
            .collect();
        assert!(
            encode(&Reply::Detail {
                detail: Detail {
                    collection: smart,
                    items: vec![],
                    seen
                },
                changed: false
            })
            .unwrap()
            .len()
                < limits::COLLECTION_REPLY_BYTES
        );
    }

    #[test]
    fn replies_must_match_the_frozen_collection_and_never_repeat_a_directory_id() {
        let value = collection();
        let reply = Reply::Detail {
            detail: Detail {
                collection: value.clone(),
                items: vec![],
                seen: vec![],
            },
            changed: false,
        };
        assert!(reply.matches(&Command::Show { id: id(1) }));
        assert!(!reply.matches(&Command::Show { id: id(2) }));
        assert!(!reply.matches(&Command::List {}));
        assert!(
            !Reply::Directory {
                collections: vec![value.clone(), value]
            }
            .valid()
        );
    }
}
