use super::*;
use std::io::Write;

/// The JSON report. `status` is the attempt's status: `sent` means the send
/// attempt completed, never that delivery was confirmed. Only a completed
/// response confirms it, and `deliveryState: "uncertain"` always accompanies a
/// channel write that gave no receipt.
pub(super) fn json_document(
    correlation: Correlation,
    response: Option<FinalResponse>,
) -> serde_json::Value {
    let mut value =
        serde_json::json!({"requestId": correlation.request_id, "target": correlation.target});
    if !correlation.inbox {
        value["pane"] = correlation.pane.clone().into();
    }
    if let Some(identity) = correlation.identity {
        value["identity"] =
            serde_json::json!({"name": identity.name, "canonicalName": identity.canonical_name});
        if correlation.inbox {
            value["recipientIdentityId"] = identity.id.into();
        }
    }
    if let Some(response) = response {
        value["status"] = "completed".into();
        value["response"] = response.body.into();
        value["bodyBytes"] = response.body_bytes.into();
        value["submittedAtMs"] = response.submitted_at_ms.into();
    } else {
        value["status"] = if correlation.inbox { "queued" } else { "sent" }.into();
        if correlation.offline {
            value["offline"] = true.into();
        }
    }
    if correlation.delivery_uncertain {
        value["deliveryState"] = "uncertain".into();
    }
    value
}

/// The primary line when a write reached a channel that gives no receipt: the
/// request was handed over, and nothing confirms that anyone received it.
pub(super) fn unconfirmed_handoff(correlation: &Correlation) -> String {
    format!(
        "Handed request {} to {} ({}); delivery is unconfirmed",
        correlation.request_id, correlation.target, correlation.pane
    )
}

pub(super) fn publish(report: Report, mode: OutputMode) -> io::Result<u8> {
    let correlation = report.correlation;
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    let terminal = stdout.terminal();
    if mode.json {
        writeln!(stdout, "{}", json_document(correlation, report.response))?;
    } else {
        if let Some(response) = report.response {
            let text = if correlation.inbox {
                format!(
                    "Completed queued request {} for {}",
                    correlation.request_id, correlation.target
                )
            } else {
                format!(
                    "Completed request {} for {} ({})",
                    correlation.request_id, correlation.target, correlation.pane
                )
            };
            tmt_cli_style::message::success(&mut stdout, terminal, &text)?;
            if correlation.delivery_uncertain {
                tmt_cli_style::message::hint(
                    &mut stdout,
                    terminal,
                    "delivery was unconfirmed at send time; this reply confirms it",
                )?;
            }
            // The responder's exact text, never styled or escaped.
            writeln!(stdout, "{}", response.body)?;
        } else if correlation.offline {
            writeln!(
                stdout,
                "{} is offline; the request is kept in Inbox ({}).",
                correlation.target, correlation.request_id
            )?;
        } else if correlation.inbox {
            tmt_cli_style::message::success(
                &mut stdout,
                terminal,
                &format!(
                    "Queued request {} for {}",
                    correlation.request_id, correlation.target
                ),
            )?;
        } else if correlation.delivery_uncertain {
            tmt_cli_style::message::warning(
                &mut stdout,
                terminal,
                &unconfirmed_handoff(&correlation),
                Some("the channel gives no receipt; do not resend"),
            )?;
        } else {
            tmt_cli_style::message::success(
                &mut stdout,
                terminal,
                &format!(
                    "Sent request {} to {} ({})",
                    correlation.request_id, correlation.target, correlation.pane
                ),
            )?;
        }
        tmt_cli_style::message::hint(
            &mut stdout,
            terminal,
            &format!(
                "retrieve it later with tmt result {}",
                correlation.request_id
            ),
        )?;
    }
    Ok(0)
}
