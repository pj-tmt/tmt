//! `tmt send-now`: take one held or waiting message out of the Digest hold and type it now
//! (#4378). Core decides who may ask and what the message's state allows; this command runs
//! the same wake an ordinary talk runs and reports what happened, never a guess.

use crate::{
    identity_context,
    invocation::{Invocation, OutputMode},
    output::{Failure, after_cleanup},
};
use serde_json::json;
use std::{
    io::{self, Write},
    time::Duration,
};
use tmt_adapters::{
    config::{ConfigFiles, ConfigPaths},
    delivery::Delivery,
    send_now::{self, SendNow},
    storage::{Storage, StorageError},
};
use tmt_cli_style::message;
use tmt_core::request::{
    RequestError,
    delivery::{DeliveryState, SendNowRejection},
};

/// What a send-now ended as. Only `Sent` and `Already` are successes.
enum Report {
    Sent,
    Already(DeliveryState),
}

fn unavailable(error: impl std::error::Error + 'static) -> Failure {
    Failure::new("SEND_NOW_ERROR", "Could not send the message now.", 1).caused_by(error)
}

fn rejection(reason: SendNowRejection, request_id: &str) -> Failure {
    let failure = Failure::new(reason.code(), reason.message(), 1);
    match reason {
        SendNowRejection::Unconfirmed => {
            failure.suggestion(format!("Inspect with 'tmt result {}'.", request_id))
        }
        _ => failure,
    }
}

fn failure(error: RequestError<StorageError>, request_id: &str) -> Failure {
    match error {
        RequestError::SendNow(reason) => rejection(reason, request_id),
        RequestError::NotFound => Failure::new(
            "SEND_NOW_NOT_FOUND",
            "That request was not found or is no longer retained.",
            3,
        ),
        other => unavailable(other),
    }
}

/// A delivery that typed nothing keeps the request queued and reopenable; one that may have
/// typed it is never retried.
fn delivery_failure(delivery: Delivery, request_id: &str) -> Failure {
    let (code, message) = send_now::untyped_failure(&delivery);
    let failure = Failure::new(code, message, 1);
    if code == "DELIVERY_UNCERTAIN" {
        failure.suggestion(format!("Inspect with 'tmt result {}'.", request_id))
    } else {
        failure.suggestion(format!(
            "Nothing was typed; retry 'tmt send-now {}' after it continues.",
            request_id
        ))
    }
}

fn run(request: Invocation) -> Result<Report, Failure> {
    let Invocation::SendNow {
        request_id,
        identity,
    } = request
    else {
        unreachable!("send-now dispatch only accepts send-now");
    };
    let selected = identity_context::required(identity.as_deref())?;
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let settings = ConfigFiles {
        paths: paths.clone(),
    }
    .load()
    .map_err(Failure::from)?
    .settings;
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "Nothing was sent; retrying the identical command is safe.",
            "SEND_NOW_ERROR",
            "Could not send the message now.",
        )
    })?;
    let pending = (|| {
        let me = identity_context::resolve(&mut storage, selected)?;
        let delay = Duration::from_secs_f64(settings.paste_enter_delay_ms / 1000.0);
        match send_now::send_now(&mut storage, &request_id, &me.id, delay)
            .map_err(|error| failure(error, &request_id))?
        {
            SendNow::Typed => Ok(Report::Sent),
            SendNow::Already(state) => Ok(Report::Already(state)),
            SendNow::Untyped(other) => Err(delivery_failure(other, &request_id)),
            SendNow::Closed => Err(rejection(SendNowRejection::Closed, &request_id)),
        }
    })();
    after_cleanup(pending, || storage.close())
}

pub fn execute(request: Invocation, mode: OutputMode) -> io::Result<u8> {
    let request_id = match &request {
        Invocation::SendNow { request_id, .. } => request_id.clone(),
        _ => unreachable!("send-now dispatch only accepts send-now"),
    };
    let report = match run(request) {
        Ok(report) => report,
        Err(error) => return error.publish(mode),
    };
    let (status, state, line) = match &report {
        Report::Sent => ("sent", "delivered", "Sent now."),
        Report::Already(state) => (
            "already",
            state.as_str(),
            "It already reached the agent, or is on its way; it was not sent again.",
        ),
    };
    let mut stdout = tmt_cli_style::stream::stdout(mode.json);
    if mode.json {
        writeln!(
            stdout,
            "{}",
            json!({"status": status, "requestId": request_id, "state": state})
        )?;
    } else {
        let terminal = stdout.terminal();
        message::success(&mut stdout, terminal, line)?;
    }
    Ok(0)
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core("Inspect with 'tmt result {}'.", &["'."], &[]),
    crate::cli_style_tests::HintSpec::core(
        "Nothing was typed; retry 'tmt send-now {}' after it continues.",
        &["' after"],
        &[],
    ),
];
