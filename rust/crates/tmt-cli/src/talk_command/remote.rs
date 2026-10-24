//! `tmt talk --remote`: one request to an agent on a Remote peer, handed to Remote's
//! `message --json` command (`contracts/remote-message-v1.md`). Core prepares and records the
//! request and owns its receipts; Remote owns the wire. Nothing here is retried: a request whose
//! delivery is unknown stays uncertain.

use crate::{
    identity_context,
    invocation::{Invocation, OutputMode},
    output::{Failure, after_cleanup},
};
use std::{
    ffi::OsStr,
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::{ConfigFiles, ConfigPaths},
    host::{CallerEnvironment, Host},
    process::{CommandRunner, UnixCommandRunner},
    remote_message::{self, RemoteMessage, RemoteMessageOutcome},
    reply_receipt::encode_route_receipt,
    request_runtime::{request_ids, wall_time_ms},
    storage::{Storage, StorageError},
};
use tmt_core::{
    exact_text::validate_exact_text,
    request::{
        AttemptStatus, Originator, PrepareRequest, RequestError, RequestKind, RequestOrigin,
        RequestRoute, RequestService, Settlement,
    },
    retention::{REQUEST_MIN_EXPIRY_MS, checked_deadline},
    settings::Settings,
};

/// How long Core waits for Remote to answer before the request is recorded as uncertain.
const SEND_BUDGET: Duration = Duration::from_secs(60);
const REF_MAX_BYTES: usize = 64;
const NAME_MAX_BYTES: usize = 128;

struct Input {
    remote: String,
    target: String,
    expect_name: Option<String>,
    message: String,
    originator: Option<String>,
}

/// What a finished send tells the user.
#[derive(Debug)]
struct Report {
    request_id: String,
    remote: String,
    target_id: Option<String>,
    target_name: String,
    status: AttemptStatus,
}

pub(crate) fn execute(request: Invocation, mode: OutputMode) -> io::Result<u8> {
    let Invocation::TalkRemote {
        remote,
        target,
        expect_name,
        message,
        originator,
    } = request
    else {
        unreachable!("remote talk dispatch")
    };
    let input = Input {
        remote,
        target,
        expect_name,
        message,
        originator,
    };
    let result = environment().and_then(|(paths, settings, program)| {
        run(
            &input,
            &paths,
            &settings,
            &program,
            &UnixCommandRunner,
            SEND_BUDGET,
        )
    });
    match result {
        Ok(report) => publish(report, mode),
        Err(error) => error.publish(mode),
    }
}

/// Everything resolved before a request is prepared: a missing or incompatible Remote refuses
/// here, so nothing is stored for a send that could not start.
fn environment() -> Result<(ConfigPaths, Settings, std::path::PathBuf), Failure> {
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let settings = ConfigFiles {
        paths: paths.clone(),
    }
    .load()
    .map_err(Failure::from)?
    .settings;
    let search = std::env::var_os("PATH").unwrap_or_default();
    let program = tmt_adapters::extension_command::resolve("remote", &search)
        .map_err(|error| {
            Failure::new("REMOTE_UNAVAILABLE", "Could not look for Remote.", 1).caused_by(error)
        })?
        .ok_or_else(unavailable)?;
    let prefix = tmt_adapters::native_install::default_install_prefix().map_err(|error| {
        Failure::new(
            "REMOTE_UNAVAILABLE",
            "Could not find the TMT installation.",
            1,
        )
        .caused_by(error)
    })?;
    let protocols = tmt_adapters::native_install::message_protocols(
        tmt_adapters::native_install::Product::Remote,
        &prefix,
    )
    .map_err(|_| unavailable())?;
    if !protocols.contains(&remote_message::PROTOCOL) {
        return Err(unavailable());
    }
    Ok((paths, settings, program))
}

fn unavailable() -> Failure {
    Failure::new(
        "REMOTE_UNAVAILABLE",
        "Remote is not installed or does not support this message protocol; no message was sent.",
        1,
    )
    .suggestion("Install or upgrade Remote with TMT's extension install and upgrade commands, then run the command again.".into())
}

fn run(
    input: &Input,
    paths: &ConfigPaths,
    settings: &Settings,
    program: &Path,
    runner: &dyn CommandRunner,
    budget: Duration,
) -> Result<Report, Failure> {
    validate(input)?;
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "No message was sent; retrying the identical command is safe.",
            "REQUEST_STATE_ERROR",
            "Could not open request storage; no message was sent.",
        )
    })?;
    let result = send(
        &mut storage,
        input,
        settings,
        program.as_os_str(),
        runner,
        budget,
    );
    after_cleanup(result, || storage.close())
}

fn validate(input: &Input) -> Result<(), Failure> {
    let invalid = |message: &str| Failure::new("REMOTE_TARGET_INVALID", message, 1);
    if input.remote.is_empty()
        || input.remote.len() > REF_MAX_BYTES
        || input.remote.chars().any(char::is_control)
    {
        return Err(invalid(
            "The Remote peer name must be 1 to 64 bytes without controls.",
        ));
    }
    let name = input.expect_name.as_deref().unwrap_or(&input.target);
    if name.is_empty() || name.len() > NAME_MAX_BYTES || name.chars().any(char::is_control) {
        return Err(invalid(
            "The target name must be 1 to 128 bytes without controls.",
        ));
    }
    validate_exact_text(input.message.as_bytes())
        .map(|_| ())
        .map_err(|_| {
            Failure::new(
                "REQUEST_INPUT_TOO_LARGE",
                "Original request exceeds the UTF-8 byte limit.",
                1,
            )
        })
}

fn send(
    storage: &mut Storage,
    input: &Input,
    settings: &Settings,
    program: &OsStr,
    runner: &dyn CommandRunner,
    budget: Duration,
) -> Result<Report, Failure> {
    let host = Host::for_caller(&CallerEnvironment::current());
    let (originator, sender) =
        identity_context::optional(storage, &host, input.originator.as_deref())?
            .map(|identity| {
                let originator = if input.originator.is_some() {
                    Originator::Explicit(identity.id.clone())
                } else {
                    Originator::Verified(identity.id.clone())
                };
                (originator, identity)
            })
            .ok_or_else(|| {
                Failure::new(
                    "REMOTE_SENDER_REQUIRED",
                    "A Remote message needs a confirmed sender; no message was sent.",
                    1,
                )
                .suggestion(
                    "Run it from an agent pane, or pass --identity with an existing identity."
                        .into(),
                )
            })?;
    let (target_id, target_name) = match &input.expect_name {
        Some(name) => (Some(input.target.clone()), name.clone()),
        None => (None, input.target.clone()),
    };
    let route = RequestRoute::Remote {
        remote: input.remote.clone(),
        target_id: target_id.clone(),
        target_name: target_name.clone(),
        sender_name: sender.name.clone(),
    };
    let (request_id, attempt_id) = request_ids();
    let expires_at_ms = checked_deadline(
        wall_time_ms(),
        (budget.as_millis() as u64 + 1000).max(REQUEST_MIN_EXPIRY_MS),
    )
    .ok_or_else(|| {
        Failure::new(
            "CONFIG_ERROR",
            "Request expiry is outside the supported range.",
            1,
        )
    })?;
    let state_error = |error: RequestError<StorageError>, request: Option<&str>| {
        let failure = Failure::new(
            "REQUEST_STATE_ERROR",
            "Could not record the request state.",
            1,
        )
        .caused_by(error);
        match request {
            Some(id) => failure.with_request(id.to_owned(), None),
            None => failure,
        }
    };
    let mut service = RequestService::new(storage, wall_time_ms);
    service
        .prepare(
            PrepareRequest {
                room_id: None,
                kind: RequestKind::Request,
                origin: RequestOrigin::Person,
                request_id: request_id.clone(),
                message: input.message.clone(),
                route: route.clone(),
                wait: false,
                expires_at_ms,
                originator,
                recipient_identity_id: None,
                preamble: None,
            },
            attempt_id.clone(),
            settings.retention_days,
        )
        .map_err(|error| state_error(error, None))?;
    let receipt = encode_route_receipt(&request_id, &attempt_id, &route);
    let created_at_ms = service
        .request_detail(&request_id)
        .map_err(|error| state_error(error, Some(&request_id)))?
        .item
        .prepared_at_ms;
    service
        .begin_send(&attempt_id)
        .map_err(|error| state_error(error, Some(&request_id)))?;
    let uuid = request_id.strip_prefix("req_").unwrap_or(&request_id);
    let outcome = remote_message::send(
        runner,
        program,
        &RemoteMessage {
            remote: &input.remote,
            request_uuid: uuid,
            created_at_ms,
            from_id: &sender.id,
            from_name: &sender.name,
            target_id: target_id.as_deref(),
            target_name: &target_name,
            message: &input.message,
            reply_request_id: &request_id,
            reply_receipt: &receipt,
        },
        Instant::now() + budget,
    );
    let (settlement, failure) = settlement(&outcome);
    service
        .settle(&attempt_id, settlement)
        .map_err(|error| state_error(error, Some(&request_id)))?;
    if let Some(failure) = failure {
        let failure = failure.with_request(request_id, None);
        return Err(if settlement == Settlement::Uncertain {
            failure.with_delivery_state("uncertain")
        } else {
            failure
        });
    }
    Ok(Report {
        request_id,
        remote: input.remote.clone(),
        target_id,
        target_name,
        status: match settlement {
            Settlement::Held => AttemptStatus::Queued,
            _ => AttemptStatus::Sent,
        },
    })
}

/// How an outcome settles the attempt, and the failure it reports, if any. Only what Remote
/// proves was not delivered is a definite failure; anything that may have been sent is uncertain.
fn settlement(outcome: &RemoteMessageOutcome) -> (Settlement, Option<Failure>) {
    let coded = |code: &str, message: &str, reason: &Option<String>| {
        Failure::new(
            code,
            match reason {
                Some(reason) => format!("{message} ({reason})"),
                None => message.to_owned(),
            },
            1,
        )
    };
    match outcome {
        RemoteMessageOutcome::Sent => (Settlement::Sent, None),
        RemoteMessageOutcome::Held => (Settlement::Held, None),
        RemoteMessageOutcome::Refused { reason } => (
            Settlement::DefinitelyFailed,
            Some(coded("REMOTE_REFUSED", "The peer refused the message; nothing was delivered.", reason)),
        ),
        RemoteMessageOutcome::Unavailable { reason } => (
            Settlement::DefinitelyFailed,
            Some(coded("REMOTE_UNAVAILABLE", "The peer is unavailable; nothing was delivered.", reason)),
        ),
        RemoteMessageOutcome::NotSent { code } => (
            Settlement::DefinitelyFailed,
            Some(Failure::new(
                code.clone(),
                "Remote refused to send the message; nothing was delivered.",
                1,
            )),
        ),
        RemoteMessageOutcome::Unconfirmed { reason } => (
            Settlement::Uncertain,
            Some(
                coded("REMOTE_UNCONFIRMED", "The peer may have received the message.", reason)
                    .suggestion("Do not resend. Inspect the request with its result command; a reply arrives by the normal receipt.".into()),
            ),
        ),
        RemoteMessageOutcome::Unknown { detail } => (
            Settlement::Uncertain,
            Some(
                Failure::new(
                    "REMOTE_UNCONFIRMED",
                    format!("{detail}; the message may have been sent."),
                    1,
                )
                .suggestion("Do not resend. Inspect the request with its result command.".into()),
            ),
        ),
    }
}

fn publish(report: Report, mode: OutputMode) -> io::Result<u8> {
    let held = report.status == AttemptStatus::Queued;
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    let terminal = stdout.terminal();
    if mode.json {
        let mut target = serde_json::json!({"name": report.target_name});
        if let Some(id) = &report.target_id {
            target["id"] = id.clone().into();
        }
        writeln!(
            stdout,
            "{}",
            serde_json::json!({
                "status": if held { "queued" } else { "sent" },
                "requestId": report.request_id,
                "remote": report.remote,
                "target": target,
            })
        )?;
    } else {
        let text = if held {
            format!(
                "Queued request {} for {} on {}; the peer delivers it later",
                report.request_id, report.target_name, report.remote
            )
        } else {
            format!(
                "Sent request {} to {} on {}",
                report.request_id, report.target_name, report.remote
            )
        };
        tmt_cli_style::message::success(&mut stdout, terminal, &text)?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests;
