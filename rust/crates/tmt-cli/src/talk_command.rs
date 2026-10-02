//! Native talk composition over existing request, identity and transport owners.

mod observation;
mod preparation;
mod presentation;
#[cfg(test)]
mod tests;

use crate::{
    invocation::{Invocation, OutputMode, TalkOptions},
    output::{Failure, after_cleanup},
};
use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::{ConfigFiles, ConfigPaths},
    host::{CallerEnvironment, Host},
    interrupt::Interrupt,
    request_runtime::wall_time_ms,
    storage::{Storage, StorageError},
};
use tmt_core::{
    exact_text::validate_exact_text,
    identity::Identity,
    request::{FinalResponse, RequestEndpoint, RequestError, RequestService, Settlement},
    settings::Settings,
};

struct Input {
    target: String,
    message: String,
    originator: Option<String>,
    options: TalkOptions,
}
#[derive(Clone)]
struct Correlation {
    data_dir: PathBuf,
    request_id: String,
    target: String,
    pane: String,
    identity: Option<Identity>,
    inbox: bool,
    offline: bool,
    /// The message reached a one-way channel that gives no receipt: it is
    /// neither confirmed nor safe to resend, and every later failure says so.
    delivery_uncertain: bool,
}
struct Prepared {
    correlation: Correlation,
    attempt_id: String,
    endpoint: Option<RequestEndpoint>,
    payload: String,
    previous_request_id: Option<String>,
    notify_originator: bool,
    wake: bool,
}
struct Report {
    correlation: Correlation,
    response: Option<FinalResponse>,
}

/// Records what a send attempt left behind. A channel write without a receipt is
/// noted on the correlation first, so even a failure to settle it reports that
/// delivery is uncertain and must not be resent.
fn settle_delivery(
    correlation: &mut Correlation,
    outcome: &crate::delivery::Delivery,
    settle: impl FnOnce(tmt_core::request::WakeState) -> Result<(), RequestError<StorageError>>,
) -> Result<(), Failure> {
    if matches!(outcome, crate::delivery::Delivery::Unacknowledged) {
        correlation.delivery_uncertain = true;
    }
    let settlement = outcome.wake_state();
    settle(settlement).map_err(|error| {
        correlation.state_error(
            error,
            settlement != tmt_core::request::WakeState::Unavailable,
        )
    })
}

impl Correlation {
    fn error(&self, code: &'static str, message: impl Into<String>, status: u8) -> Failure {
        let failure = self.correlated(Failure::new(code, message, status));
        if self.inbox {
            failure.with_inbox_target(&self.target, self.identity.as_ref())
        } else {
            failure.with_target(&self.target, &self.pane, self.identity.as_ref())
        }
    }
    fn inspection(&self) -> String {
        if self.inbox {
            format!(
                "Inspect with 'tmt result {}'. Do not resend solely because the observer ended.",
                self.request_id
            )
        } else {
            format!(
                "Inspect with 'tmt result {}' and 'tmt check {}' before deciding whether to retry.",
                self.request_id, self.target
            )
        }
    }
    /// Ties a failure to this request and keeps the channel's uncertainty on it.
    fn correlated(&self, failure: Failure) -> Failure {
        let failure = failure.with_request(self.request_id.clone(), None);
        if self.delivery_uncertain {
            failure.with_delivery_state("uncertain")
        } else {
            failure
        }
    }

    fn state_error(&self, error: RequestError<StorageError>, possible_delivery: bool) -> Failure {
        let error = match error {
            RequestError::Repository(storage) => {
                return self
                    .correlated(Failure::storage_access(
                        storage,
                        &self.data_dir,
                        if self.delivery_uncertain {
                            "The channel write may have been delivered; do not resend. Inspect the retained request."
                        } else if possible_delivery {
                            "Pane input may have happened; inspect the retained request before retrying."
                        } else {
                            "No message was sent; inspect the retained request before retrying."
                        },
                        "REQUEST_STATE_ERROR",
                        if possible_delivery {
                            "Request state failed after transport; delivery may have occurred."
                        } else {
                            "Request state failed before transport; no message was sent."
                        },
                    ))
                    .suggestion(self.inspection());
            }
            other => other,
        };
        self.error(
            "REQUEST_STATE_ERROR",
            if possible_delivery {
                "Request state failed after transport; delivery may have occurred."
            } else {
                "Request state failed before transport; no message was sent."
            },
            1,
        )
        .suggestion(self.inspection())
        .caused_by(error)
    }
    fn interrupted(&self) -> Failure {
        self.error(
            "INTERRUPTED",
            "Interrupted while waiting for a durable reply.",
            1,
        )
        .suggestion(self.inspection())
    }

    fn socket_error(&self, error: tmt_adapters::host::DeliveryError) -> Failure {
        self.error(
            "TMUX_PERMISSION_DENIED",
            "TMT cannot access the tmux socket. An agent sandbox may be blocking it: allow the socket or use the provider's escalation to inspect the retained request. No message was sent; inspect before retrying.",
            1,
        )
        .caused_by(error)
    }
}

fn deliver(
    storage: &mut Storage,
    prepared: &mut Prepared,
    input: &Input,
    settings: &Settings,
    interrupt: Option<&Interrupt>,
) -> Result<Option<FinalResponse>, Failure> {
    let correlation = &mut prepared.correlation;
    let wait = !input.options.detach && !correlation.offline;
    let timeout = input.options.timeout_seconds.unwrap_or(settings.timeout);
    let deadline = Instant::now() + Duration::from_secs_f64(timeout);
    if prepared.notify_originator {
        let waiter = if wait {
            match tmt_adapters::process::runtime::observe_runtime_process(
                &tmt_adapters::process::SupervisedProbeRunner,
                u64::from(std::process::id()),
                Instant::now() + Duration::from_secs(1),
            ) {
                Ok(tmt_adapters::process::runtime::ProcessObservation::Live(value)) => Some(value),
                _ => None,
            }
        } else {
            None
        };
        RequestService::new(&mut *storage, wall_time_ms)
            .enable_notifications(
                &correlation.request_id,
                tmt_core::request::notification::NotificationPolicy {
                    deadline_ms: wall_time_ms() + (timeout * 1000.0).ceil() as u64,
                    timeout_ms: (timeout * 1000.0).ceil() as u64,
                    waiter,
                },
            )
            .map_err(|error| correlation.state_error(error, false))?;
    }
    if prepared.wake {
        RequestService::new(&mut *storage, wall_time_ms)
            .queue(&prepared.attempt_id)
            .map_err(|error| correlation.state_error(error, false))?;
    }
    let pending = (|| {
        if interrupt.is_some_and(Interrupt::is_interrupted) {
            // Inbox publication already committed during preparation. Stopping
            // the sender's wait must not retract a recipient's queued request.
            if prepared.endpoint.is_some() {
                RequestService::new(&mut *storage, wall_time_ms)
                    .settle(&prepared.attempt_id, Settlement::DefinitelyFailed)
                    .map_err(|error| correlation.state_error(error, false))?;
            }
            return Err(correlation.interrupted());
        }
        // A pane that resolved to no active identity is pasted to directly, but that
        // does not prove it never opted into a channel: existing evidence (or being
        // unable to tell) stops it before anything is attempted.
        // A pane no identity claims is pasted to directly, behind the same check as
        // every baseline paste: the drivers' own enrollment records for this pane.
        if let Some(endpoint) = &prepared.endpoint
            && correlation.identity.is_none()
        {
            let pane = tmt_adapters::runtime::channel::PaneAddress {
                server: &endpoint.server,
                pane_id: &endpoint.pane_id,
                pane_pid: endpoint.pane_pid,
            };
            match crate::delivery::pane_channel_evidence(
                &pane,
                None,
                &tmt_adapters::config::ConfigPaths::channel_directory_in(&correlation.data_dir),
            ) {
                Ok(skipped) => {
                    for record in skipped {
                        warn_unattributed_record(&record);
                    }
                }
                Err(evidence) => {
                    RequestService::new(&mut *storage, wall_time_ms)
                        .settle(&prepared.attempt_id, Settlement::DefinitelyFailed)
                        .map_err(|error| correlation.state_error(error, false))?;
                    return Err(correlation
                        .error(evidence.fault.error_code(), nothing_pasted(&evidence), 1)
                        .suggestion(correlation.inspection()));
                }
            }
        }
        if prepared.endpoint.is_some()
            && let Err(primary) =
                RequestService::new(&mut *storage, wall_time_ms).begin_send(&prepared.attempt_id)
        {
            // Settlement is idempotent; the primary failure remains diagnostic.
            let primary = correlation.state_error(primary, false);
            return Err(
                match RequestService::new(&mut *storage, wall_time_ms)
                    .settle(&prepared.attempt_id, Settlement::DefinitelyFailed)
                {
                    Ok(()) => primary,
                    Err(secondary) => primary.with_secondary_error(secondary),
                },
            );
        }
        if prepared.wake
            && let Some(identity) = &correlation.identity
        {
            let claim = RequestService::new(&mut *storage, wall_time_ms)
                .claim_wake(&correlation.request_id)
                .map_err(|error| correlation.state_error(error, false))?;
            if !claim.claimed {
                correlation.inbox = true;
                return RequestService::new(&mut *storage, wall_time_ms)
                    .get_response(&correlation.request_id)
                    .map(|value| match value {
                        tmt_core::request::ResponseLookup::Available(response) => Some(*response),
                        _ => None,
                    })
                    .map_err(|error| correlation.state_error(error, false));
            }
            let eligible = RequestService::new(&mut *storage, wall_time_ms)
                .wake_recipient_is_eligible(&correlation.request_id, &identity.id)
                .map_err(|error| correlation.state_error(error, false))?;
            // An offline acceptance is queue-only for this request, even if
            // the endpoint comes back during publication. Never re-wake it.
            let outcome = if correlation.offline {
                crate::delivery::Delivery::Offline
            } else if eligible {
                let attempt = crate::delivery::send(
                    storage,
                    &identity.id,
                    &prepared.payload,
                    Duration::from_secs_f64(settings.paste_enter_delay_ms / 1000.0),
                )
                .map_err(|error| {
                    Failure::storage_access(
                        error,
                        &correlation.data_dir,
                        "No message was sent; inspect the retained request before retrying.",
                        "DELIVERY_PREPARATION_FAILED",
                        "Could not verify delivery state.",
                    )
                })?;
                for record in &attempt.unattributed {
                    warn_unattributed_record(record);
                }
                attempt.delivery
            } else {
                crate::delivery::Delivery::Unavailable
            };
            let request_id = correlation.request_id.clone();
            settle_delivery(correlation, &outcome, |settlement| {
                RequestService::new(&mut *storage, wall_time_ms)
                    .settle_request_delivery(&request_id, settlement)
            })?;
            if matches!(outcome, crate::delivery::Delivery::Offline) {
                correlation.offline = true;
                correlation.inbox = true;
                return Ok(None);
            }
            if matches!(outcome, crate::delivery::Delivery::Unacknowledged) {
                // Settled as an uncertain wake above; keep waiting for the
                // durable reply without resending or pasting.
            } else if let crate::delivery::Delivery::ChannelUnavailable(evidence) = &outcome {
                // The session opted into a channel that cannot carry this
                // request. Only a session that never opted in is pasted to.
                return Err(correlation
                    .error(evidence.fault.error_code(), nothing_pasted(evidence), 1)
                    .suggestion(format!(
                        "Check the session, then retry later; the request stays queued. {}",
                        correlation.inspection()
                    )));
            } else if matches!(outcome, crate::delivery::Delivery::AwaitingApproval) {
                // The agent refused the prompt while it waits on its user;
                // nothing reached the pane and nothing types around it.
                let name = correlation
                    .identity
                    .as_ref()
                    .map_or(correlation.target.as_str(), |identity| {
                        identity.name.as_str()
                    });
                return Err(correlation
                    .error(
                        "DELIVERY_AWAITING_APPROVAL",
                        format!("{name} is waiting on its user; nothing was sent."),
                        1,
                    )
                    .suggestion(format!(
                        "Retry after it continues; the request stays queued. {}",
                        correlation.inspection()
                    )));
            } else if !matches!(outcome, crate::delivery::Delivery::Sent) {
                if let crate::delivery::Delivery::Transport(error) = outcome {
                    if error.socket_permission_denied() {
                        return Err(correlation.socket_error(error));
                    }
                    return Err(correlation
                        .error(
                            if error.uncertain() {
                                "DELIVERY_UNCERTAIN"
                            } else {
                                "DELIVERY_PREPARATION_FAILED"
                            },
                            error.to_string(),
                            1,
                        )
                        .at_stage(error.stage.as_str())
                        .suggestion(correlation.inspection())
                        .caused_by(error));
                }
                return Err(correlation.error(
                    if matches!(outcome, crate::delivery::Delivery::Uncertain) {
                        "DELIVERY_UNCERTAIN"
                    } else {
                        "DELIVERY_PREPARATION_FAILED"
                    },
                    "Recipient delivery did not complete; inspect the retained request.",
                    1,
                ));
            }
        } else if let Some(endpoint) = &prepared.endpoint {
            let delivered = Host::for_server(&endpoint.server).send(
                endpoint,
                &prepared.payload,
                Duration::from_secs_f64(settings.paste_enter_delay_ms / 1000.0),
            );
            match delivered {
                Ok(()) => RequestService::new(&mut *storage, wall_time_ms)
                    .settle(&prepared.attempt_id, Settlement::Sent)
                    .map_err(|error| correlation.state_error(error, true))?,
                Err(error) => {
                    let uncertain = error.uncertain();
                    if let Err(state) = RequestService::new(&mut *storage, wall_time_ms).settle(
                        &prepared.attempt_id,
                        if uncertain {
                            Settlement::Uncertain
                        } else {
                            Settlement::DefinitelyFailed
                        },
                    ) {
                        return Err(correlation
                            .state_error(state, uncertain)
                            .with_secondary_error(error));
                    }
                    if error.socket_permission_denied() {
                        return Err(correlation.socket_error(error));
                    }
                    return Err(correlation
                        .error(
                            if uncertain {
                                "DELIVERY_UNCERTAIN"
                            } else {
                                "DELIVERY_PREPARATION_FAILED"
                            },
                            error.to_string(),
                            1,
                        )
                        .at_stage(error.stage.as_str())
                        .suggestion(correlation.inspection())
                        .caused_by(error));
                }
            }
        }
        if !wait {
            return Ok(None);
        }
        observation::observe(
            || {
                RequestService::new(&mut *storage, wall_time_ms)
                    .get_response(&correlation.request_id)
                    .and_then(|lookup| match lookup {
                        tmt_core::request::ResponseLookup::Available(response) => {
                            Ok(Some(*response))
                        }
                        tmt_core::request::ResponseLookup::Unavailable => Ok(None),
                        tmt_core::request::ResponseLookup::NotRequired => {
                            Err(tmt_core::request::RequestError::StateInvalid)
                        }
                    })
                    .map_err(|error| correlation.state_error(error, true))
            },
            correlation,
            deadline,
            timeout,
            settings.poll_interval,
            interrupt.expect("wait owns interrupt guard"),
        )
        .map(Some)
    })();
    let released = if wait {
        RequestService::new(&mut *storage, wall_time_ms)
            .finish_wait(&prepared.attempt_id, matches!(&pending, Ok(Some(_))))
            .map_err(|error| correlation.state_error(error, true))
    } else {
        Ok(None)
    };
    if let Ok(Some(hint)) = &released {
        crate::delivery::notify(storage, hint);
    }
    // Waiter release always runs, including rejected transport and read errors.
    match pending {
        Err(primary) => Err(match released {
            Ok(_) => primary,
            Err(secondary) => primary.with_secondary_error(secondary),
        }),
        Ok(value) => released.map(|_| value),
    }
}

fn run(
    input: Input,
    paths: ConfigPaths,
    settings: Settings,
    interrupt: Option<&Interrupt>,
    mode: OutputMode,
) -> Result<Report, Failure> {
    let mut storage = Storage::open(&paths.database).map_err(|error| {
        Failure::storage_access(
            error,
            &paths.global_dir,
            "No message was sent; retrying the identical command is safe.",
            "REQUEST_STATE_ERROR",
            "Could not open request storage; no message was sent.",
        )
    })?;
    // The originator is caller-scoped; the recipient's own host delivers.
    let host = Host::for_caller(&CallerEnvironment::current());
    let mut cleanup_correlation = None;
    let pending = preparation::prepare(&mut storage, &host, &input, &settings, interrupt, &paths.global_dir).and_then(|mut prepared| {
        cleanup_correlation = Some(prepared.correlation.clone());
        if !mode.json && !input.options.detach && !input.options.force && let Some(previous) = &prepared.previous_request_id {
            let mut stderr = tmt_cli_style::stream::stderr();
            let terminal = stderr.terminal();
            let _ = tmt_cli_style::message::warning(&mut stderr, terminal, &format!("Another recent request exists for '{}' (id: {previous}). Input processing is not serialized; durable results remain associated by request ID.", input.target), None);
        }
        let response = deliver(&mut storage, &mut prepared, &input, &settings, interrupt);
        if response.is_ok() && prepared.notify_originator
            && prepared.correlation.offline && !input.options.detach
            && let Err(error) = crate::request_observer_command::start(&paths.database, &prepared.correlation.request_id) {
            let mut stderr = tmt_cli_style::stream::stderr();
            let terminal = stderr.terminal();
            let _ = tmt_cli_style::message::warning(&mut stderr, terminal, &format!("Timeout notification unavailable ({error}); the request is retained."), Some(&format!("do not resend; inspect it with tmt result {}", prepared.correlation.request_id)));
        }
        let correlation = prepared.correlation;
        response.map(|response| Report { correlation, response })
    });
    after_cleanup(pending, || storage.close()).map_err(|error| {
        if error.code == "CLEANUP_ERROR"
            && let Some(correlation) = cleanup_correlation
        {
            let error = error.with_request(correlation.request_id, None);
            if correlation.inbox {
                error.with_inbox_target(&correlation.target, correlation.identity.as_ref())
            } else {
                error.with_target(
                    &correlation.target,
                    &correlation.pane,
                    correlation.identity.as_ref(),
                )
            }
        } else {
            error
        }
    })
}

pub fn execute(request: Invocation, mode: OutputMode) -> io::Result<u8> {
    let Invocation::Talk {
        target,
        message,
        originator,
        options,
    } = request
    else {
        unreachable!("talk dispatch")
    };
    let input = Input {
        target,
        message,
        originator,
        options,
    };
    let preflight: Result<_, Failure> = (|| {
        let paths = ConfigPaths::discover().map_err(Failure::from)?;
        let settings = ConfigFiles {
            paths: paths.clone(),
        }
        .load()
        .map_err(Failure::from)?
        .settings;
        validate_exact_text(input.message.as_bytes()).map_err(|_| {
            Failure::new(
                "REQUEST_INPUT_TOO_LARGE",
                "Original request exceeds the UTF-8 byte limit.",
                1,
            )
        })?;
        Ok((paths, settings))
    })();
    let (paths, settings) = match preflight {
        Ok(value) => value,
        Err(error) => return error.publish(mode),
    };
    // Keep callbacks alive through waiter/storage cleanup and output. A second
    // SIGINT retains emergency termination if synchronous work cannot finish.
    let interrupt = if input.options.detach {
        None
    } else {
        match Interrupt::install() {
            Ok(guard) => Some(guard),
            Err(error) => {
                return Failure::new(
                    "ERROR",
                    "Could not install observer interruption handling.",
                    1,
                )
                .caused_by(error)
                .publish(mode);
            }
        }
    };
    match run(input, paths, settings, interrupt.as_ref(), mode) {
        Ok(report) => presentation::publish(report, mode),
        Err(error) => error.publish(mode),
    }
}

/// What both paste sites tell the user when channel evidence stops a paste: the
/// driver's reason with the record at fault and its recovery.
fn nothing_pasted(evidence: &tmt_adapters::runtime::channel::EvidenceError) -> String {
    format!(
        "{} Nothing was sent and nothing was pasted.",
        evidence.message()
    )
}

/// A channel record that names no pane (unreadable, or written by an older `tmt`)
/// protects and blocks nothing; it is reported so it can be removed.
fn warn_unattributed_record(record: &std::path::Path) {
    let mut stderr = tmt_cli_style::stream::stderr();
    let terminal = stderr.terminal();
    let _ = tmt_cli_style::message::warning(
        &mut stderr,
        terminal,
        &format!(
            "Skipped channel record {}: it names no pane, so it cannot protect or block this one.",
            record.display()
        ),
        Some("delete it if its session is gone"),
    );
}
