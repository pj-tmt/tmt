//! `tmt inbox` and `tmt answer`: the recipient's side of a request, selected
//! by identity instead of a receipt. Selection and the proof stay in
//! RequestService; the proof never reaches output.

use crate::{
    identity_context,
    invocation::{Invocation, OutputMode},
    output::{Failure, after_cleanup, identity_document, identity_missing},
    response_command::{body, response_failure},
};
use serde_json::{Value, json};
use std::io::{self, Write};
use tmt_adapters::{
    config::ConfigPaths,
    request_runtime::wall_time_ms,
    storage::{Storage, StorageError},
};
use tmt_cli_style::{
    Token,
    list::{self, Section},
    message,
    table::{Cell, Column, Table},
    value,
};
use tmt_core::{
    identity::Identity,
    request::{
        FinalResponse, RequestError, RequestService, SubmitResponse,
        attention::AttentionRejection,
        inbox::{AnswerRejection, OpenPage, OpenRequest},
        notification::NotificationOutcome,
    },
};

enum Report {
    Inbox {
        page: OpenPage,
        /// Originator identity ID to its current identity.
        names: Vec<(String, Identity)>,
    },
    Answered {
        /// None for an anonymous originator.
        from: Option<Identity>,
        response: Box<FinalResponse>,
        notification: Option<NotificationOutcome>,
    },
}

fn unavailable(error: impl std::error::Error + 'static) -> Failure {
    Failure::new("ANSWER_ERROR", "Could not read or answer requests.", 1).caused_by(error)
}

fn preview(item: &OpenRequest) -> String {
    item.preview
        .as_deref()
        .and_then(|text| text.lines().next())
        .unwrap_or("(prompt no longer retained)")
        .to_owned()
}

fn answer_failure(error: RequestError<StorageError>, from: &str, paths: &ConfigPaths) -> Failure {
    match error {
        RequestError::Answer(reason) => match &reason {
            AnswerRejection::NotWaiting => Failure::new(
                reason.code(),
                format!("{from} is not waiting on you for a response."),
                3,
            )
            .suggestion("tmt inbox".into()),
            AnswerRejection::Ambiguous(items) => {
                let mut text = format!(
                    "{from} is waiting on you for {} requests; nothing was sent.",
                    items.len()
                );
                for item in items {
                    text.push_str(&format!("\n  {}  {}", item.request_id, preview(item)));
                }
                Failure::new(reason.code(), text, 1)
                    .suggestion("choose one with --request <request-id>".into())
            }
        },
        RequestError::Attention(AttentionRejection::NotFound) => Failure::new(
            "X_NOT_FOUND",
            if from.is_empty() {
                "That request is not addressed to you.".to_owned()
            } else {
                format!("That request is not one {from} sent you.")
            },
            3,
        ),
        RequestError::Invalid(message) => Failure::new("ANSWER_INPUT_INVALID", message, 1),
        other => response_failure(other, &paths.global_dir, true),
    }
}

fn open(paths: &ConfigPaths) -> Result<Storage, Failure> {
    Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "Nothing was stored; retrying the identical command is safe.",
            "ANSWER_ERROR",
            "Could not read or answer requests.",
        )
    })
}

fn identity(storage: &Storage, name: &str) -> Result<Identity, Failure> {
    storage
        .resolve_identity(name)
        .map_err(unavailable)?
        .ok_or_else(|| identity_missing(name))
}

fn run(request: Invocation) -> Result<(Identity, Report), Failure> {
    let selected = match &request {
        Invocation::Inbox { identity, .. } | Invocation::Answer { identity, .. } => {
            identity_context::required(identity.as_deref())?
        }
        _ => unreachable!("answer dispatch only accepts inbox/answer"),
    };
    run_selected(None, selected, request)
}

fn run_selected(
    paths: Option<&ConfigPaths>,
    selected: identity_context::Selector,
    request: Invocation,
) -> Result<(Identity, Report), Failure> {
    // Read the body before storage, as `reply` does.
    let content = match &request {
        Invocation::Answer { input, .. } => Some(body(input.clone())?),
        _ => None,
    };
    let discovered;
    let paths = match paths {
        Some(paths) => paths,
        None => {
            discovered = ConfigPaths::discover().map_err(unavailable)?;
            &discovered
        }
    };
    let mut storage = open(paths)?;
    let pending = (|| {
        let me = identity_context::resolve(&mut storage, selected)?;
        let report = match request {
            Invocation::Inbox { from, limit, .. } => {
                let from = from.map(|name| identity(&storage, &name)).transpose()?;
                let page = RequestService::new(&mut storage, wall_time_ms)
                    .open_requests(&me.id, from.as_ref().map(|from| from.id.as_str()), limit)
                    .map_err(|error| answer_failure(error, "", paths))?;
                let mut names: Vec<(String, Identity)> = Vec::new();
                for id in page
                    .items
                    .iter()
                    .filter_map(|item| item.originator.identity_id())
                {
                    if names.iter().all(|(known, _)| known != id)
                        && let Some(found) = storage.find_identity_by_id(id).map_err(unavailable)?
                    {
                        names.push((id.to_owned(), found));
                    }
                }
                Report::Inbox { page, names }
            }
            Invocation::Answer { from, request, .. } => {
                let named = from.map(|name| identity(&storage, &name)).transpose()?;
                let label = named
                    .as_ref()
                    .map_or(String::new(), |from| from.canonical_name.clone());
                let fail = |error| answer_failure(error, &label, paths);
                let (request_id, proof, originator) =
                    RequestService::new(&mut storage, wall_time_ms)
                        .answer_target(
                            &me.id,
                            named.as_ref().map(|from| from.id.as_str()),
                            request.as_deref(),
                        )
                        .map_err(fail)?;
                let from = match (named, originator.identity_id()) {
                    (Some(named), _) => Some(named),
                    (None, Some(id)) => storage.find_identity_by_id(id).map_err(unavailable)?,
                    (None, None) => None,
                };
                let gone_waiter = tmt_adapters::delivery::gone_waiter(&mut storage, &request_id);
                let (response, hint) = RequestService::new(&mut storage, wall_time_ms)
                    .submit_response_with_hint(
                        SubmitResponse {
                            request_id,
                            proof,
                            body: content.expect("answer reads its body first"),
                        },
                        gone_waiter.as_ref(),
                    )
                    .map_err(fail)?;
                let notification = hint
                    .as_ref()
                    .map(|hint| crate::reply_notice_command::notify(&mut storage, hint));
                Report::Answered {
                    from,
                    response: Box::new(response),
                    notification,
                }
            }
            _ => unreachable!(),
        };
        Ok((me, report))
    })();
    after_cleanup(pending, || storage.close())
}

fn from_document(item: &OpenRequest, names: &[(String, Identity)]) -> Value {
    let Some(id) = item.originator.identity_id() else {
        return Value::Null;
    };
    match names.iter().find(|(known, _)| known == id) {
        Some((_, found)) => party(found),
        None => json!({"identityId": id, "name": null, "canonicalName": null}),
    }
}

/// The shape `x listen` uses for a sender or recipient.
fn party(identity: &Identity) -> Value {
    json!({"identityId": identity.id, "name": identity.name, "canonicalName": identity.canonical_name})
}

fn document(me: &Identity, report: &Report) -> Value {
    let mut value = json!({"identity": identity_document(me)});
    match report {
        Report::Inbox { page, names } => {
            value["items"] = page
                .items
                .iter()
                .map(|item| {
                    let mut row = json!({
                        "requestId": item.request_id,
                        "from": from_document(item, names),
                        "preparedAtMs": item.prepared_at_ms,
                        "delivery": item.delivery.as_str(),
                        "preview": item.preview,
                    });
                    if let Some(room) = &item.room_id {
                        row["roomId"] = json!(room);
                    }
                    row
                })
                .collect();
            value["more"] = json!(page.more);
        }
        Report::Answered {
            from,
            response,
            notification,
        } => {
            value["status"] = json!("submitted");
            value["requestId"] = json!(response.request_id);
            value["from"] = from.as_ref().map_or(Value::Null, party);
            value["bodyBytes"] = json!(response.body_bytes);
            value["submittedAtMs"] = json!(response.submitted_at_ms);
            if let Some(notification) = notification {
                value["notification"] = json!(notification.as_str());
            }
        }
    }
    value
}

pub fn execute(request: Invocation, mode: OutputMode) -> io::Result<u8> {
    let (me, report) = match run(request) {
        Ok(done) => done,
        Err(error) => return error.publish(mode),
    };
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        writeln!(stdout, "{}", document(&me, &report))?;
        return Ok(0);
    }
    let terminal = stdout.terminal();
    match report {
        Report::Inbox { page, .. } if page.items.is_empty() => {
            writeln!(stdout, "Nothing is waiting on you.")?;
        }
        Report::Inbox { page, names } => {
            let now = wall_time_ms();
            let mut rows =
                Table::new(&[Column::Fixed, Column::Fixed, Column::Fixed, Column::Detail]);
            for item in &page.items {
                let from = from_document(item, &names);
                rows.row([
                    Cell::from(from["canonicalName"].as_str().unwrap_or("anonymous")),
                    Cell::styled(
                        value::relative_time(now.saturating_sub(item.prepared_at_ms)),
                        Token::Dim,
                    ),
                    // Whole: `answer --request` takes it.
                    Cell::styled(&item.request_id, Token::Dim),
                    Cell::from(preview(item)),
                ]);
            }
            let note = page
                .more
                .then_some("newer requests not shown; raise --limit");
            list::write(
                &mut stdout,
                terminal,
                &[Section {
                    title: "waiting on you",
                    count: Some(page.items.len()),
                    rows,
                    note,
                    hint: Some("tmt answer <name> \"…\""),
                }],
            )?;
        }
        Report::Answered {
            from,
            response,
            notification,
        } => message::success(
            &mut stdout,
            terminal,
            &format!(
                "Answered {} ({}){}",
                from.as_ref()
                    .map_or("an anonymous sender", |from| from.canonical_name.as_str()),
                response.request_id,
                notification
                    .map(|value| format!("; originator notification {}", value.as_str()))
                    .unwrap_or_default()
            ),
        )?,
    }
    Ok(0)
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core("tmt answer <name> \"…\"", &[""], &[]),
    crate::cli_style_tests::HintSpec::core("tmt inbox", &[""], &[]),
];

pub(crate) fn inbox_document(
    paths: &ConfigPaths,
    identity: &str,
    from: Option<String>,
    limit: Option<u64>,
) -> Result<Value, Failure> {
    run_selected(
        Some(paths),
        identity_context::Selector::SavedId(identity.into()),
        Invocation::Inbox {
            identity: Some(identity.into()),
            from,
            limit,
        },
    )
    .map(|(me, report)| document(&me, &report))
}
