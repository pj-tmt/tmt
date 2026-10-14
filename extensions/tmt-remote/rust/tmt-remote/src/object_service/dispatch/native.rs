//! Native calls share the existing bounded queue, workers, bus and stop owner.
use super::{Bus, ChannelEnd, Fault, Hub, SLICE, Stage};
use crate::{objects::IoBudget, sealed_sync::SyncError};
use std::{sync::atomic::Ordering, time::Instant};
use tmt_extension_objects::{ChannelFrame, sync as wire};
fn failure(request: &wire::Frame, hub: &Hub, error: SyncError) -> wire::Frame {
    wire::Frame::Result {
        version: 1,
        generation: hub.generation.to_string(),
        request_id: request
            .request_id()
            .expect("admitted request ID")
            .to_string(),
        method: request.method(),
        outcome: super::super::sync::failure(error),
    }
}
pub(super) fn serve(
    hub: &Hub,
    bus: &Bus,
    request: &wire::Frame,
    deadline: Instant,
) -> Result<(), ChannelEnd> {
    #[cfg(test)]
    if let Some(hook) = &hub.hook {
        hook(super::Pause::BeforeRequest, deadline)
    }
    let io = IoBudget {
        deadline,
        cancelled: &hub.stop,
    };
    let reply = match &hub.native {
        Some(native) => match super::super::sync::complete(native.execute(request, &io), &io) {
            Ok(reply) => reply,
            Err(SyncError::Cancelled | SyncError::Deadline)
                if request.method() == wire::Method::Publish =>
            {
                wire::Frame::Result {
                    version: 1,
                    generation: hub.generation.to_string(),
                    request_id: request
                        .request_id()
                        .expect("admitted request ID")
                        .to_string(),
                    method: wire::Method::Publish,
                    outcome: wire::Outcome::Published {
                        status: wire::Published::Unknown,
                    },
                }
            }
            Err(error) => failure(request, hub, error),
        },
        None => failure(request, hub, SyncError::Unavailable),
    };
    #[cfg(test)]
    if let Some(hook) = &hub.hook {
        hook(super::Pause::BeforeResultWrite, deadline)
    }
    if hub.stopped() {
        return Err(ChannelEnd::Stopped);
    }
    let registered = match &reply {
        wire::Frame::Result {
            outcome: wire::Outcome::Subscribed { id },
            ..
        } => Some(id.clone()),
        _ => None,
    };
    let refusal = match &reply {
        wire::Frame::Result {
            outcome: wire::Outcome::Error { error },
            ..
        } => Some(error.clone()),
        _ => None,
    };
    let sent = bus.send_channel_until(&ChannelFrame::Sync(reply), deadline);
    match sent {
        Err(Fault::Timeout(Stage::Write)) if bus.fault().is_none() => {
            if let (Some(native), Some(id)) = (&hub.native, &registered) {
                native.cancel_registration(id)
            }
            let mut fallback = failure(request, hub, SyncError::Deadline);
            if let wire::Frame::Result { outcome, .. } = &mut fallback {
                if let Some(error) = refusal {
                    *outcome = wire::Outcome::Error { error }
                } else if request.method() == wire::Method::Publish {
                    *outcome = wire::Outcome::Published {
                        status: wire::Published::Unknown,
                    }
                }
            }
            bus.send_channel(&ChannelFrame::Sync(fallback))
                .map_err(ChannelEnd::Bus)?;
        }
        Err(error) => return Err(ChannelEnd::Bus(error)),
        Ok(()) => {
            if let (Some(native), Some(id)) = (&hub.native, registered) {
                native.registered(&id)
            }
        }
    }
    Ok(())
}
pub(super) fn announce(hub: &Hub, bus: &Bus) {
    let Some(native) = &hub.native else { return };
    while !hub.stop.load(Ordering::Acquire) {
        if let Some(notice) = native.next_notice(Instant::now() + SLICE) {
            if hub.stopped() {
                return;
            }
            if let Err(error) = bus.send_channel(&ChannelFrame::Sync(notice)) {
                // Cancel's result already removed this interest: a taken-but-unsent
                // notice must not end its otherwise healthy channel.
                if error == Fault::Correlation(tmt_extension_objects::Reason::Unknown)
                    && bus.fault().is_none()
                {
                    continue;
                }
                hub.end(ChannelEnd::Bus(error));
            }
        }
    }
}
