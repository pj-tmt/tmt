use super::*;
use crate::{identity_context, target};
use tmt_adapters::{reply_receipt::encode_route_receipt, request_runtime::request_ids};
use tmt_core::{
    profile::{ProfileKind, ProfileReader},
    request::{Originator, PreambleReservation, PrepareRequest, RequestRoute},
    retention::{REQUEST_MIN_EXPIRY_MS, checked_deadline},
    settings::PreambleMode,
};

pub(super) fn prepare(
    storage: &mut Storage,
    host: &Host,
    input: &Input,
    settings: &Settings,
    interrupt: Option<&Interrupt>,
    data_dir: &std::path::Path,
) -> Result<Prepared, Failure> {
    let room = input
        .options
        .room
        .as_deref()
        .map(|selector| crate::room_command::resolve(storage, selector))
        .transpose()?;
    let mut offline = false;
    let automatic = if !input.options.inbox && !tmt_core::names::is_pane_target(&input.target) {
        let identity = match tmt_core::identity::find_by_name(storage, &input.target) {
            Ok(value) => value,
            // Preserve target resolution's existing missing-name boundary.
            Err(tmt_core::identity::IdentityError::InvalidName(_)) => None,
            Err(error) => {
                return Err(Failure::new(
                    "IDENTITY_ERROR",
                    "Could not read recipient identity.",
                    1,
                )
                .caused_by(error));
            }
        };
        if let Some(identity) = identity {
            offline = matches!(
                crate::delivery::status(storage, &identity.id).map_err(|error| {
                    Failure::new(
                        "DELIVERY_PREPARATION_FAILED",
                        "Could not read recipient state.",
                        1,
                    )
                    .caused_by(error)
                })?,
                crate::delivery::Availability::Offline
            );
            Some(identity)
        } else {
            None
        }
    } else {
        None
    };
    let inbox_identity = if input.options.inbox {
        Some(identity_context::resolve(
            storage,
            identity_context::Selector::Explicit(input.target.clone()),
        )?)
    } else {
        automatic
    };
    let observed = if input.options.inbox || offline {
        None
    } else {
        Some(target::resolve(storage, host, &input.target).map_err(|error| {
            if error.code == "NAME_NOT_FOUND"
                && tmt_core::identity::find_by_name(storage, &input.target)
                    .ok()
                    .flatten()
                    .is_some_and(|identity| {
                        identity.lifetime == tmt_core::identity::Lifetime::Saved
                    })
            {
                error.suggestion(
                    "For a saved identity without an active pane, use `tmt talk <identity> <message> --inbox` for durable delivery.".into(),
                )
            } else {
                error
            }
        })?)
    };
    let (originator, sender) =
        identity_context::optional(storage, host, input.originator.as_deref())?.map_or(
            (Originator::Unknown, "unknown".to_owned()),
            |identity| {
                let originator = if input.originator.is_some() {
                    Originator::Explicit(identity.id)
                } else {
                    Originator::Verified(identity.id)
                };
                (originator, identity.name)
            },
        );
    let (request_id, attempt_id) = request_ids();
    let correlation = Correlation {
        data_dir: data_dir.to_path_buf(),
        request_id,
        target: input.target.clone(),
        pane: observed
            .as_ref()
            .map(|value| value.pane.id.clone())
            .unwrap_or_default(),
        identity: inbox_identity
            .clone()
            .or_else(|| observed.as_ref().and_then(|value| value.identity.clone())),
        inbox: input.options.inbox || offline,
        offline,
    };
    if let Some(room) = &room
        && !correlation
            .identity
            .as_ref()
            .is_some_and(|identity| room.member_ids.contains(&identity.id))
    {
        return Err(correlation.error(
            "ROOM_RECIPIENT_NOT_MEMBER",
            "The direct target must be an identified member of the selected room. No message was sent.",
            1,
        ));
    }
    if let Some(delay) = input.options.delay_seconds {
        let delay = Duration::from_secs_f64(delay);
        if let Some(interrupt) = interrupt {
            interrupt
                .wait_until(Instant::now() + delay)
                .map_err(|error| {
                    correlation
                        .error("ERROR", "Could not wait before delivery.", 1)
                        .caused_by(error)
                })?;
        } else {
            std::thread::sleep(delay);
        }
    }
    if interrupt.is_some_and(Interrupt::is_interrupted) {
        return Err(correlation.interrupted());
    }
    let preamble = if input.options.inbox
        || input.options.no_preamble
        || settings.preamble_mode == PreambleMode::Disabled
        || settings.preamble_every == 0
    {
        None
    } else if !offline && let Some(identity) = correlation.identity.as_ref() {
        storage
            .find_profile(&identity.id, ProfileKind::Preamble)
            .map_err(|error| {
                Failure::new("PREAMBLE_ERROR", "Could not read recipient preamble.", 1)
                    .caused_by(error)
            })?
            .filter(|profile| !profile.content.is_empty())
            .map(|profile| (identity.id.clone(), profile.content))
    } else {
        None
    };
    let route = match (&correlation.identity, &observed) {
        (Some(identity), _) => RequestRoute::Inbox {
            recipient_identity_id: identity.id.clone(),
        },
        (_, Some(observed)) => RequestRoute::Pane(target::refresh(observed)?),
        _ => unreachable!("one route is selected"),
    };
    let timeout_ms = if input.options.detach {
        0
    } else {
        (input.options.timeout_seconds.unwrap_or(settings.timeout) * 1000.0).ceil() as u64
    };
    let budget = timeout_ms + settings.paste_enter_delay_ms.ceil() as u64 + 1000;
    let expires_at_ms = checked_deadline(wall_time_ms(), budget.max(REQUEST_MIN_EXPIRY_MS))
        .ok_or_else(|| {
            correlation.error(
                "CONFIG_ERROR",
                "Request expiry is outside the supported range.",
                1,
            )
        })?;
    let notify_originator = originator.identity_id().is_some() && !input.options.inbox;
    let request = PrepareRequest {
        room_id: room.map(|room| room.id),
        kind: tmt_core::request::RequestKind::Request,
        request_id: correlation.request_id.clone(),
        message: input.message.clone(),
        route: route.clone(),
        wait: !input.options.detach && !offline,
        expires_at_ms,
        originator,
        recipient_identity_id: correlation
            .identity
            .as_ref()
            .map(|identity| identity.id.clone()),
        preamble: (!input.options.inbox)
            .then_some(preamble.as_ref())
            .flatten()
            .as_ref()
            .map(|(identity_id, _)| PreambleReservation {
                identity_id: identity_id.clone(),
                every: settings.preamble_every,
            }),
    };
    let mut service = RequestService::new(storage, wall_time_ms);
    let prepared = if input.options.inbox {
        service.enqueue(request, attempt_id.clone(), settings.retention_days)
    } else {
        service.prepare(request, attempt_id.clone(), settings.retention_days)
    }
    .map_err(|error| correlation.state_error(error, false))?;
    let receipt = encode_route_receipt(&correlation.request_id, &attempt_id, &route);
    let message = if prepared.inject_preamble {
        preamble.map_or_else(
            || input.message.clone(),
            |(_, content)| format!("[SYSTEM: {content}]\n\n{}", input.message),
        )
    } else {
        input.message.clone()
    };
    let payload = format!(
        "{message}\n\n<tmt-reply from=\"{}\">\ntmt reply {} --receipt {receipt} --message <text>\n</tmt-reply>\nSubmit your response with the command above. Chat output alone does not complete the request. After successful submission, show a brief summary; report submission errors.",
        escape_sender_attribute(&sender),
        correlation.request_id
    );
    let wake = !input.options.inbox && correlation.identity.is_some();
    Ok(Prepared {
        correlation,
        attempt_id,
        endpoint: match route {
            RequestRoute::Pane(endpoint) => Some(endpoint),
            RequestRoute::Inbox { .. } => None,
        },
        payload,
        previous_request_id: prepared.previous_request_id,
        notify_originator,
        wake,
    })
}

// Presentation only: keep identity resolution and stored message bytes unchanged.
fn escape_sender_attribute(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for character in name.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::escape_sender_attribute;

    #[test]
    fn sender_attribute_preserves_plain_names_and_escapes_input_once() {
        assert_eq!(escape_sender_attribute("Alice Team"), "Alice Team");
        assert_eq!(escape_sender_attribute("unknown"), "unknown");
        assert_eq!(
            escape_sender_attribute("A&\"<'>&amp;"),
            "A&amp;&quot;&lt;&apos;&gt;&amp;amp;"
        );
        assert_eq!(
            escape_sender_attribute("\"><tmt-reply from=\"other"),
            "&quot;&gt;&lt;tmt-reply from=&quot;other"
        );
    }
}
