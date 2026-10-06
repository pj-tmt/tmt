//! Durable response composition with independent best-effort reply hints. Input
//! before storage; all final-response decisions remain in RequestService.

use crate::{
    invocation::{ContentInput, Invocation, OutputMode},
    output::{Failure, after_cleanup},
};
use serde_json::json;
use std::{
    error::Error,
    io::{self, Write},
    path::Path,
};
use tmt_adapters::{
    config::ConfigPaths,
    reply_receipt::decode_reply_receipt,
    request_runtime::wall_time_ms,
    response_input::{ResponseInputError, ResponseInputFailure, read_file, read_stdin},
    storage::{Storage, StorageError},
};
use tmt_core::{
    exact_text::{MAX_EXCHANGE_TEXT_BYTES, validate_exact_text},
    request::{
        FinalResponse, RequestError, RequestService, ResponseLookup, ResponseRejection,
        ResultSelectionRejection, SubmitResponse,
    },
};

enum Report {
    Withdrawn(String, tmt_core::request::Withdrawal),
    NotRequired(String),
    Submitted(
        FinalResponse,
        Option<tmt_core::request::notification::NotificationOutcome>,
    ),
    Completed(FinalResponse),
}

fn unavailable(error: impl Error + 'static) -> Failure {
    Failure::new(
        "RESPONSE_ERROR",
        "Could not complete the response operation.",
        1,
    )
    .caused_by(error)
}

fn input_failure(error: ResponseInputError) -> Failure {
    let status = if error.kind == ResponseInputFailure::Timeout {
        4
    } else {
        1
    };
    Failure::new(error.code(), error.to_string(), status).caused_by(error)
}

pub(crate) fn response_failure(
    error: RequestError<StorageError>,
    directory: &Path,
    is_reply: bool,
) -> Failure {
    let error = match error {
        RequestError::Repository(storage) => {
            return Failure::storage_access(
                storage,
                directory,
                if is_reply {
                    "Nothing was stored; retrying the identical reply command is safe."
                } else {
                    "No result was changed; retrying the identical command is safe."
                },
                "RESPONSE_ERROR",
                "Could not complete the response operation.",
            );
        }
        other => other,
    };
    if let RequestError::ResultSelection(reason) = &error {
        let message = match reason {
            ResultSelectionRejection::PrefixTooShort => error.to_string(),
            ResultSelectionRejection::Ambiguous(matches) => {
                let more = matches.total.saturating_sub(matches.ids.len() as u64);
                format!(
                    "Request-ID prefix is ambiguous: {}{}",
                    matches.ids.join(", "),
                    if more > 0 {
                        format!("; …and {more} more")
                    } else {
                        String::new()
                    }
                )
            }
        };
        return Failure::new("USAGE_ERROR", message, 1).caused_by(error);
    }
    let RequestError::Response(reason) = &error else {
        return unavailable(error);
    };
    let (status, message) = match reason {
        ResponseRejection::Withdrawn => (5, "Request was withdrawn by its originator."),
        ResponseRejection::NotRequired => (1, "Announcements do not accept replies."),
        ResponseRejection::InputInvalid => (1, "Response input is invalid."),
        ResponseRejection::InputTooLarge => (1, "Response body exceeds the UTF-8 byte limit."),
        ResponseRejection::RequestNotFound => (3, "Request was not found."),
        ResponseRejection::AttemptMismatch => (1, "Response attempt does not match the request."),
        ResponseRejection::RecipientMismatch => {
            (1, "Response does not match the original recipient.")
        }
        ResponseRejection::ReceiptMismatch => (1, "Response receipt does not match the request."),
        ResponseRejection::StateInvalid => (
            1,
            "Final response cannot be accepted in the current request state.",
        ),
        ResponseRejection::Conflict => (5, "Request already has a different final response."),
        ResponseRejection::Expired => (
            1,
            "Final response can no longer be accepted or is no longer retained.",
        ),
    };
    Failure::new(reason.code(), message, status).caused_by(error)
}

pub(crate) fn body(input: ContentInput) -> Result<String, Failure> {
    match input {
        ContentInput::Inline(text) => {
            validate_exact_text(text.as_bytes()).map_err(|_| {
                Failure::new(
                    "RESPONSE_INPUT_TOO_LARGE",
                    format!("Response body must not exceed {MAX_EXCHANGE_TEXT_BYTES} UTF-8 bytes."),
                    1,
                )
            })?;
            Ok(text)
        }
        ContentInput::File(path) => read_file(Path::new(&path)).map_err(input_failure),
        ContentInput::Stdin => read_stdin().map_err(input_failure),
    }
}

fn run(request: Invocation) -> Result<Report, Failure> {
    run_at(None, request)
}

fn run_at(paths: Option<&ConfigPaths>, request: Invocation) -> Result<Report, Failure> {
    // A prepared input is not a parallel service request: use the core owner.
    let (request_id, submission) = match request {
        Invocation::Reply {
            request_id,
            receipt,
            input,
        } => {
            let proof = decode_reply_receipt(&receipt, &request_id).map_err(|error| {
                Failure::new(error.code(), error.to_string(), 1).caused_by(error)
            })?;
            let body = body(input)?;
            let input = SubmitResponse {
                request_id: request_id.clone(),
                proof,
                body,
            };
            (request_id, Some(input))
        }
        Invocation::Result { request_id } => (request_id, None),
        _ => unreachable!("response dispatch only accepts reply/result"),
    };
    let discovered;
    let paths = match paths {
        Some(paths) => paths,
        None => {
            discovered = ConfigPaths::discover().map_err(unavailable)?;
            &discovered
        }
    };
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            if submission.is_some() {
                "Nothing was stored; retrying the identical reply command is safe."
            } else {
                "No result was changed; retrying the identical command is safe."
            },
            "RESPONSE_ERROR",
            "Could not complete the response operation.",
        )
    })?;
    // Invalid clocks fail closed through the service's safe-integer validation.
    // Sample inside its transaction, not once before acquiring the writer lock.
    let pending = match submission {
        Some(input) => {
            let gone_waiter = crate::delivery::gone_waiter(&mut storage, &request_id);
            let accepted = RequestService::new(&mut storage, wall_time_ms)
                .submit_response_with_hint(input, gone_waiter.as_ref())
                .map_err(|error| response_failure(error, &paths.global_dir, true));
            accepted.map(|(response, hint)| {
                let notification = hint
                    .as_ref()
                    .map(|hint| crate::reply_notice_command::notify(&mut storage, hint));
                Report::Submitted(response, notification)
            })
        }
        None => RequestService::new(&mut storage, wall_time_ms)
            .get_response_by_prefix(&request_id)
            .map_err(|error| response_failure(error, &paths.global_dir, false))
            .and_then(|(resolved_id, record)| match record {
                ResponseLookup::Withdrawn(withdrawal) => {
                    Ok(Report::Withdrawn(resolved_id, withdrawal))
                }
                ResponseLookup::Available(response) => Ok(Report::Completed(*response)),
                ResponseLookup::NotRequired => Ok(Report::NotRequired(resolved_id.clone())),
                ResponseLookup::Unavailable => Err(Failure::new(
                    "RESPONSE_NOT_AVAILABLE",
                    format!("Response for request '{resolved_id}' is not available."),
                    3,
                )
                .with_request(resolved_id, Some("unavailable"))),
            }),
    };
    after_cleanup(pending, || storage.close()).map_err(|error| {
        if error.code == "CLEANUP_ERROR" {
            error.with_request(request_id, None)
        } else {
            error
        }
    })
}

pub fn execute(request: Invocation, mode: OutputMode) -> io::Result<u8> {
    let report = match run(request) {
        Ok(report) => report,
        Err(error) => return error.publish(mode),
    };
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    let terminal = stdout.terminal();
    match report {
        report if mode.json => writeln!(stdout, "{}", document(&report))?,
        Report::Withdrawn(request_id, withdrawal) => writeln!(
            stdout,
            "Request '{request_id}' withdrawn at {}: {}",
            withdrawal.withdrawn_at_ms, withdrawal.reason
        )?,
        Report::NotRequired(request_id) => writeln!(
            stdout,
            "Announcement '{request_id}' does not require a response."
        )?,
        Report::Submitted(record, notification) => tmt_cli_style::message::success(
            &mut stdout,
            terminal,
            &format!(
                "Submitted response for request {} ({} bytes){}",
                record.request_id,
                record.body_bytes,
                notification
                    .map(|value| format!("; originator notification {}", value.as_str()))
                    .unwrap_or_default()
            ),
        )?,
        Report::Completed(record) => write_result(&mut stdout, &record.request_id, &record.body)?,
    }
    Ok(0)
}

/// `tmt result`: a header, then the stored response byte for byte. The body is
/// the responder's exact text, so it is never styled or escaped.
fn write_result(output: &mut impl Write, request_id: &str, body: &str) -> io::Result<()> {
    writeln!(output, "Response for request '{request_id}':\n{body}")
}

fn document(report: &Report) -> serde_json::Value {
    match report {
        Report::Withdrawn(request_id, withdrawal) => {
            json!({"status":"withdrawn", "requestId":request_id, "reason":withdrawal.reason, "withdrawnAtMs":withdrawal.withdrawn_at_ms})
        }
        Report::NotRequired(request_id) => {
            json!({"status": "not_required", "requestId": request_id})
        }
        Report::Submitted(record, notification) => {
            let mut value = json!({"status":"submitted", "requestId":record.request_id, "bodyBytes":record.body_bytes, "submittedAtMs":record.submitted_at_ms});
            if let Some(notification) = notification {
                value["notification"] = json!(notification.as_str());
            }
            value
        }
        Report::Completed(record) => {
            json!({"status":"completed", "requestId":record.request_id, "response":record.body, "bodyBytes":record.body_bytes, "submittedAtMs":record.submitted_at_ms})
        }
    }
}

pub(crate) fn result_document(
    paths: &ConfigPaths,
    request_id: String,
) -> Result<serde_json::Value, Failure> {
    run_at(Some(paths), Invocation::Result { request_id }).map(|report| document(&report))
}
