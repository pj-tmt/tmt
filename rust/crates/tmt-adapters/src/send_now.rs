//! Send now as one operation for every caller (#4378): Core decides who may ask and what the
//! message's state allows, then the ordinary wake types the request. The CLI and the extension
//! API both call this, so neither re-states a rule.

use crate::{
    delivery::{self, Delivery, Released},
    request_runtime::wall_time_ms,
    storage::{Storage, StorageError},
};
use std::time::Duration;
use tmt_core::request::{
    RequestError, RequestService,
    delivery::{DeliveryState, SendNowOutcome, SendNowUntyped},
};

/// What a send-now ended as. Only `Typed` and `Already` are successes.
pub enum SendNow {
    /// Typed into the pane.
    Typed,
    /// It had already reached the agent, or another caller owns its wake right now.
    Already(DeliveryState),
    /// The hold is released but the paste did not complete; the delivery says why. A request
    /// whose paste was refused stays sendable, one whose paste may have happened is never retried.
    Untyped(Delivery),
    /// The request can no longer be answered, so nothing was typed.
    Closed,
}

pub fn send_now(
    storage: &mut Storage,
    request_id: &str,
    caller_id: &str,
    delay: Duration,
) -> Result<SendNow, RequestError<StorageError>> {
    let recipient =
        match RequestService::new(&mut *storage, wall_time_ms).send_now(request_id, caller_id)? {
            SendNowOutcome::Already(state) => return Ok(SendNow::Already(state)),
            SendNowOutcome::Release {
                recipient_identity_id,
            } => recipient_identity_id,
        };
    Ok(
        match delivery::deliver_released(storage, request_id, &recipient, delay)? {
            Released::Typed(Delivery::Sent) => SendNow::Typed,
            Released::Typed(other) => SendNow::Untyped(other),
            Released::NotClaimed(_) => SendNow::Already(DeliveryState::Sent),
            Released::Closed => SendNow::Closed,
        },
    )
}

/// The code and sentence for a paste that did not complete, shared by every caller.
pub fn untyped_failure(delivery: &Delivery) -> (&'static str, &'static str) {
    let untyped = match delivery {
        Delivery::AwaitingApproval => SendNowUntyped::AwaitingApproval,
        Delivery::NotAgent(_) => SendNowUntyped::NotAgent,
        Delivery::Offline | Delivery::Unavailable => SendNowUntyped::Unreachable,
        Delivery::ChannelUnavailable(_) => SendNowUntyped::ChannelUnavailable,
        Delivery::Uncertain | Delivery::Unacknowledged | Delivery::Transport(_) => {
            SendNowUntyped::Uncertain
        }
        Delivery::Sent => unreachable!("a delivered message is a success"),
    };
    (untyped.code(), untyped.message())
}
