//! Identity-scoped attention presentation over the existing request service.

mod listen;
mod presentation;

use crate::{
    identity_context,
    invocation::{ExchangeOperation, OutputMode},
    output::{Failure, after_cleanup},
};
use std::{error::Error, io};
use tmt_adapters::{
    config::ConfigPaths,
    reply_receipt::encode_route_receipt,
    request_runtime::wall_time_ms,
    storage::{Storage, StorageError},
};
use tmt_core::{
    identity::Identity,
    request::{
        RequestError, RequestService,
        attention::{Acknowledged, ExchangeDetail, ExchangePage, FinalState},
    },
};

enum ResultKind {
    List(ExchangePage),
    Show {
        detail: ExchangeDetail,
        receipt: Option<String>,
    },
    Ack(Acknowledged),
    Ackall(u64),
}

struct Report {
    identity: Identity,
    result: ResultKind,
}

fn unavailable(error: impl Error + 'static) -> Failure {
    Failure::new("X_ERROR", "Could not complete the exchange operation.", 1).caused_by(error)
}

fn request_failure(error: RequestError<StorageError>) -> Failure {
    let (code, status) = match &error {
        RequestError::Attention(reason) => (
            reason.code(),
            match reason {
                tmt_core::request::attention::AttentionRejection::NotFound => 3,
                tmt_core::request::attention::AttentionRejection::RevisionConflict { .. } => 5,
                _ => 1,
            },
        ),
        RequestError::RevisionExhausted => ("X_REVISION_EXHAUSTED", 1),
        _ => return unavailable(error),
    };
    Failure::new(code, error.to_string(), status).caused_by(error)
}

fn run(identity: Option<String>, operation: ExchangeOperation) -> Result<Report, Failure> {
    let selector = identity_context::required(identity.as_deref())?;
    let paths = ConfigPaths::discover().map_err(unavailable)?;
    run_selected(&paths, selector, operation)
}

fn run_selected(
    paths: &ConfigPaths,
    selector: identity_context::Selector,
    operation: ExchangeOperation,
) -> Result<Report, Failure> {
    let mut storage = Storage::open(&paths.database).map_err(unavailable)?;
    let pending = (|| {
        let identity = identity_context::resolve(&mut storage, selector).map_err(|error| {
            if error.code == "IDENTITY_ERROR" {
                unavailable(error)
            } else {
                error
            }
        })?;
        let mut service = RequestService::new(&mut storage, wall_time_ms);
        let result = match operation {
            ExchangeOperation::List { limit, after } => service
                .list_exchanges(&identity.id, limit, after)
                .map(ResultKind::List),
            ExchangeOperation::Show {
                request_id,
                incoming,
            } => {
                let detail = if incoming {
                    service.show_incoming_request(&identity.id, &request_id)
                } else {
                    service.show_exchange(&identity.id, &request_id)
                };
                detail.and_then(|detail| {
                    let receipt =
                        if incoming && detail.exchange.final_state != FinalState::NotRequired {
                            let context = service.get_context(&request_id)?.ok_or(
                                RequestError::Attention(
                                    tmt_core::request::attention::AttentionRejection::NotFound,
                                ),
                            )?;
                            Some(encode_route_receipt(
                                &request_id,
                                &context.attempt.attempt_id,
                                &context.attempt.route,
                            ))
                        } else {
                            None
                        };
                    Ok(ResultKind::Show { detail, receipt })
                })
            }
            ExchangeOperation::Ack {
                request_id,
                revision,
                incoming,
            } => if incoming {
                service.acknowledge_incoming_request(&identity.id, &request_id, revision)
            } else {
                service.acknowledge_exchange(&identity.id, &request_id, revision)
            }
            .map(ResultKind::Ack),
            ExchangeOperation::Ackall { incoming } => if incoming {
                service.acknowledge_all_incoming_requests(&identity.id)
            } else {
                service.acknowledge_all_exchanges(&identity.id)
            }
            .map(ResultKind::Ackall),
            ExchangeOperation::Listen { .. } => unreachable!("listen has its own bounded runner"),
        }
        .map_err(request_failure)?;
        Ok(Report { identity, result })
    })();
    after_cleanup(pending, || storage.close())
}

pub fn execute(
    identity: Option<String>,
    operation: ExchangeOperation,
    mode: OutputMode,
) -> io::Result<u8> {
    if let ExchangeOperation::Listen {
        room,
        timeout_seconds,
        debounce_seconds,
    } = &operation
    {
        return listen::execute(
            identity,
            room.clone(),
            *timeout_seconds,
            *debounce_seconds,
            mode,
        );
    }
    match run(identity, operation) {
        Ok(report) => presentation::publish(report, mode),
        Err(error) => error.publish(mode),
    }
}

#[cfg(test)]
pub(crate) use listen::PRINTED_HINTS as LISTEN_HINTS;

#[cfg(test)]
pub(crate) use presentation::PRINTED_HINTS as PRESENTATION_HINTS;

pub(crate) fn request_document(
    paths: &ConfigPaths,
    identity: &str,
    request_id: String,
) -> Result<serde_json::Value, Failure> {
    run_selected(
        paths,
        identity_context::Selector::SavedId(identity.into()),
        ExchangeOperation::Show {
            request_id,
            incoming: true,
        },
    )
    .map(|report| presentation::document(&report))
}

pub(crate) fn ack_document(
    paths: &ConfigPaths,
    identity: &str,
    request_id: String,
    revision: u64,
) -> Result<serde_json::Value, Failure> {
    run_selected(
        paths,
        identity_context::Selector::SavedId(identity.into()),
        ExchangeOperation::Ack {
            request_id,
            revision,
            incoming: true,
        },
    )
    .map(|report| presentation::document(&report))
}
