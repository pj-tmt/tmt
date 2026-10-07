use super::*;
use crate::output::identity_document;
use serde_json::{Value, json};
use std::io::Write;
use tmt_cli_style::{
    Token,
    list::{self, Section},
    message,
    table::{Cell, Column, Table},
    value,
};
use tmt_core::request::{
    RequestPrompt,
    attention::{Exchange, FinalState},
};

fn final_document<T>(state: &FinalState<T>, content: impl FnOnce(&T) -> Option<Value>) -> Value {
    match state {
        FinalState::Withdrawn(withdrawal) => {
            json!({"status":"withdrawn", "reason":withdrawal.reason, "withdrawnAtMs":withdrawal.withdrawn_at_ms})
        }
        FinalState::NotRequired => json!({"status": "not_required"}),
        FinalState::NotSubmitted => json!({"status": "not_submitted"}),
        FinalState::Expired {
            submitted_at_ms,
            expires_at_ms,
        } => {
            json!({"status": "expired", "submittedAtMs": submitted_at_ms, "expiresAtMs": expires_at_ms})
        }
        FinalState::Unavailable {
            submitted_at_ms,
            expires_at_ms,
        } => {
            json!({"status": "unavailable", "submittedAtMs": submitted_at_ms, "expiresAtMs": expires_at_ms})
        }
        FinalState::Retained {
            content: body,
            submitted_at_ms,
            body_bytes,
            expires_at_ms,
        } => {
            let mut value = json!({"status": "retained", "submittedAtMs": submitted_at_ms, "bodyBytes": body_bytes, "expiresAtMs": expires_at_ms});
            if let Some(body) = content(body) {
                value["response"] = body;
            }
            value
        }
    }
}

fn prompt_status(prompt: &RequestPrompt) -> &'static str {
    match prompt {
        RequestPrompt::Unavailable => "unavailable",
        RequestPrompt::Expired { .. } => "expired",
        RequestPrompt::Retained(_) => "retained",
    }
}

fn exchange_document<T>(
    exchange: &Exchange<T>,
    content: impl FnOnce(&T) -> Option<Value>,
) -> Value {
    let mut document = json!({
        "requestId": exchange.request_id,
        "recipientIdentityId": exchange.recipient_identity_id,
        "preparedAtMs": exchange.prepared_at_ms,
        "delivery": exchange.delivery.as_str(),
        "final": final_document(&exchange.final_state, content),
        "revision": exchange.revision,
        "acknowledged": exchange.acknowledged,
        "settled": exchange.settled,
        "retentionExpiresAtMs": exchange.retention_expires_at_ms,
    });
    if let Some(room_id) = &exchange.room_id {
        document["roomId"] = room_id.clone().into();
    }
    document
}

fn prompt_document(prompt: &RequestPrompt) -> Value {
    match prompt {
        RequestPrompt::Unavailable => json!({"status": "unavailable"}),
        RequestPrompt::Expired { expires_at_ms } => {
            json!({"status": "expired", "expiresAtMs": expires_at_ms})
        }
        RequestPrompt::Retained(prompt) => {
            json!({"status": "retained", "message": prompt.message, "messageBytes": prompt.message_bytes, "expiresAtMs": prompt.expires_at_ms})
        }
    }
}

pub(super) fn document(report: &Report) -> Value {
    let mut value = json!({"identity": identity_document(&report.identity)});
    match &report.result {
        ResultKind::Withdraw(result) => {
            value["status"] = json!("withdrawn");
            value["requestId"] = json!(result.request_id);
            value["reason"] = json!(result.withdrawal.reason);
            value["withdrawnAtMs"] = json!(result.withdrawal.withdrawn_at_ms);
            value["changed"] = json!(result.changed);
        }
        ResultKind::List(page) => {
            value["items"] = page
                .items
                .iter()
                .map(|item| exchange_document(item, |_| None))
                .collect();
            value["nextAfter"] = json!(page.next_after);
        }
        ResultKind::Show { detail, receipt } => {
            let mut exchange = exchange_document(&detail.exchange, |body| Some(json!(body)));
            exchange["prompt"] = prompt_document(&detail.prompt);
            if let Some(receipt) = receipt {
                exchange["reply"] = json!({"receipt": receipt, "command": format!("tmt reply {} --receipt {} --message <text>", detail.exchange.request_id, receipt)});
            }
            value["exchange"] = exchange;
        }
        ResultKind::Ack(ack) => {
            value["requestId"] = json!(ack.request_id);
            value["revision"] = json!(ack.revision);
            value["acknowledged"] = json!(true);
            value["changed"] = json!(ack.changed);
        }
        ResultKind::Ackall(through) => value["acknowledgedThrough"] = json!(through),
    }
    value
}

pub(super) fn publish(report: Report, mode: OutputMode) -> io::Result<u8> {
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        writeln!(stdout, "{}", document(&report))?;
        return Ok(0);
    }
    let terminal = stdout.terminal();
    match &report.result {
        ResultKind::Withdraw(result) => message::success(
            &mut stdout,
            terminal,
            &format!(
                "{} {}: {}",
                if result.changed {
                    "Withdrew"
                } else {
                    "Already withdrew"
                },
                result.request_id,
                result.withdrawal.reason
            ),
        )?,
        ResultKind::List(page) => {
            if page.items.is_empty() {
                writeln!(stdout, "No unacknowledged exchanges.")?;
            } else {
                // Request ids stay whole: `x show`, `x ack` and `result` take them.
                let mut rows = Table::new(&[
                    Column::Name,
                    Column::Fixed,
                    Column::Fixed,
                    Column::Fixed,
                    Column::Fixed,
                ]);
                for item in &page.items {
                    rows.row([
                        Cell::from(&item.request_id),
                        Cell::styled(
                            item.recipient_identity_id
                                .as_deref()
                                .map_or("-", value::short_id),
                            Token::Dim,
                        ),
                        Cell::from(item.delivery.as_str()),
                        Cell::from(item.final_state.as_str()),
                        Cell::styled(format!("r{}", item.revision), Token::Dim),
                    ]);
                }
                let next = page.next_after.map(|after| {
                    format!(
                        "more with tmt x ls --after {after} --identity {}",
                        crate::output::shell_word(&report.identity.canonical_name)
                    )
                });
                list::write(
                    &mut stdout,
                    terminal,
                    &[Section {
                        title: "exchanges",
                        count: Some(page.items.len()),
                        rows,
                        note: None,
                        hint: next.as_deref(),
                    }],
                )?;
            }
        }
        ResultKind::Show { detail, receipt } => {
            let item = &detail.exchange;
            tmt_cli_style::detail::write(
                &mut stdout,
                terminal,
                &item.request_id,
                &[
                    ("delivery", item.delivery.as_str().to_owned()),
                    ("final", item.final_state.as_str().to_owned()),
                    ("revision", item.revision.to_string()),
                    ("acknowledged", item.acknowledged.to_string()),
                    ("settled", item.settled.to_string()),
                ],
            )?;
            writeln!(
                stdout,
                "{} {}",
                terminal.paint(Token::Title, "PROMPT"),
                terminal.paint(Token::Dim, prompt_status(&detail.prompt))
            )?;
            if let RequestPrompt::Retained(prompt) = &detail.prompt {
                write_exact(&mut stdout, &prompt.message)?;
            }
            if let FinalState::Retained { content, .. } = &item.final_state {
                writeln!(stdout, "{}", terminal.paint(Token::Title, "FINAL"))?;
                write_exact(&mut stdout, content)?;
            }
            if let FinalState::Withdrawn(withdrawal) = &item.final_state {
                writeln!(
                    stdout,
                    "Withdrawn at {}: {}",
                    withdrawal.withdrawn_at_ms, withdrawal.reason
                )?;
            }
            if let Some(receipt) = receipt {
                message::hint(
                    &mut stdout,
                    terminal,
                    &format!(
                        "tmt reply {} --receipt {receipt} --message <text>",
                        item.request_id
                    ),
                )?;
            }
        }
        ResultKind::Ack(ack) if ack.changed => message::success(
            &mut stdout,
            terminal,
            &format!(
                "Acknowledged {} at revision {}",
                ack.request_id, ack.revision
            ),
        )?,
        ResultKind::Ack(ack) => writeln!(
            stdout,
            "{} was already acknowledged at revision {}",
            ack.request_id, ack.revision
        )?,
        ResultKind::Ackall(through) => message::success(
            &mut stdout,
            terminal,
            &format!(
                "Acknowledged '{}' through revision {through}; later revisions remain unacknowledged",
                report.identity.name
            ),
        )?,
    }
    Ok(0)
}

/// A prompt or final response byte for byte, like `tmt result`: it is the
/// sender's or responder's exact text, so it is never styled or escaped.
fn write_exact(output: &mut impl Write, text: &str) -> io::Result<()> {
    writeln!(output, "{text}")
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "more with tmt x ls --after {after} --identity {}",
        &[""],
        &[("{after}", "1")],
    ),
    crate::cli_style_tests::HintSpec::core(
        "tmt reply {} --receipt {receipt} --message <text>",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "tmt reply {} --receipt {} --message <text>",
        &[""],
        &[],
    ),
];
