//! Shared composition for requests and advisory hints. Drivers own IO policy.

use std::time::{Duration, Instant};
use tmt_adapters::{
    process::{SupervisedProbeRunner, runtime::observe_runtime_process},
    runtime::RuntimeRegistry,
    storage::{Storage, StorageError},
    tmux::{ActionError, BindingSession, Tmux},
};
use tmt_core::{
    binding::{
        BindingEntry, BindingRepository,
        session::{ObservedSessionKey, RuntimeLiveness, RuntimeState, SessionTransition},
    },
    driver::{
        ActionResult, Driver, InterfacePresence, InterfaceStatus, SendFailure,
        routing::send_preferred,
    },
    request::{
        RequestService, WakeState,
        notification::{HintKind, OriginatorHint},
    },
};

pub enum Delivery {
    Sent,
    Offline,
    Uncertain,
    Unavailable,
    Transport(tmt_adapters::tmux::DeliveryError),
}

impl Delivery {
    pub fn wake_state(&self) -> WakeState {
        match self {
            Self::Sent => WakeState::Sent,
            Self::Uncertain => WakeState::Uncertain,
            Self::Transport(error) if error.uncertain() => WakeState::Uncertain,
            _ => WakeState::Unavailable,
        }
    }
}

pub enum Availability {
    Ready,
    Offline,
    Unavailable,
}

pub fn current(
    storage: &mut Storage,
    identity: &str,
) -> Result<Option<BindingEntry>, StorageError> {
    storage.with_binding_transaction(|records| records.entry_by_id(identity))
}

/// Ended is not a permanent ban on this pane. A new independently verified
/// runtime can replace it; the old incarnation or a live shell cannot.
fn recover(
    storage: &mut Storage,
    registry: &RuntimeRegistry,
    entry: &mut BindingEntry,
) -> Result<(), StorageError> {
    let Some(binding) = &entry.binding else {
        return Ok(());
    };
    let preferences = storage
        .with_binding_transaction(|records| records.session_preferences(&entry.identity.id))?;
    let Some(driver) = preferences
        .preferred_harness
        .as_ref()
        .and_then(|id| registry.lifecycle(id))
    else {
        return Ok(());
    };
    let deadline = Instant::now() + Duration::from_secs(1);
    if let Some(key) = &binding.session.key {
        let gone = observe_runtime_process(&SupervisedProbeRunner, key.incarnation.pid(), deadline)
            .is_ok_and(|value| value.matches(&key.incarnation) == RuntimeLiveness::Gone);
        if !gone {
            return Ok(());
        }
    }
    let Some(process) = driver.observe_replacement(binding.pane_pid, deadline) else {
        return Ok(());
    };
    let Some(next) = binding.session.admit(
        ObservedSessionKey {
            incarnation: process,
            provider_session: None,
        },
        SessionTransition::Started,
        RuntimeLiveness::Alive,
    ) else {
        return Ok(());
    };
    let changed = storage.with_binding_transaction(|records| {
        if records.entry_by_id(&entry.identity.id)?.as_ref() != Some(entry) {
            return Ok(false);
        }
        records.set_session_state(&binding.id, &binding.session, &next)
    })?;
    if changed {
        entry.binding.as_mut().expect("verified binding").session = next;
        if let Ok(paths) = tmt_adapters::config::ConfigPaths::discover() {
            crate::pane_badge::refresh(
                &paths,
                &Tmux::default(),
                entry.binding.as_ref().expect("verified binding"),
                Instant::now() + Duration::from_secs(1),
            );
        }
    }
    Ok(())
}

pub fn status(
    storage: &mut Storage,
    tmux: &Tmux,
    identity: &str,
) -> Result<Availability, StorageError> {
    let Some(mut entry) = current(storage, identity)? else {
        return Ok(Availability::Offline);
    };
    let mut host = BindingSession::new(tmux);
    match host.status(&entry) {
        ActionResult::Completed(InterfaceStatus {
            presence: InterfacePresence::Present,
            runtime,
        }) => {
            if runtime == RuntimeState::Ended {
                recover(storage, &RuntimeRegistry::first_party(), &mut entry)?;
            }
            Ok(match host.status(&entry) {
                ActionResult::Completed(InterfaceStatus {
                    presence: InterfacePresence::Present,
                    runtime: RuntimeState::Ended,
                }) => Availability::Offline,
                ActionResult::Completed(InterfaceStatus {
                    presence: InterfacePresence::Present,
                    ..
                }) => Availability::Ready,
                _ => Availability::Unavailable,
            })
        }
        ActionResult::Completed(InterfaceStatus {
            presence: InterfacePresence::Gone,
            ..
        }) => Ok(Availability::Offline),
        _ => Ok(Availability::Unavailable),
    }
}

pub fn send(
    storage: &mut Storage,
    tmux: &Tmux,
    identity: &str,
    message: &str,
    delay: Duration,
) -> Result<Delivery, StorageError> {
    match status(storage, tmux, identity)? {
        Availability::Ready => {}
        Availability::Offline => return Ok(Delivery::Offline),
        Availability::Unavailable => return Ok(Delivery::Unavailable),
    }
    let Some(entry) = current(storage, identity)? else {
        return Ok(Delivery::Offline);
    };
    let preferences =
        storage.with_binding_transaction(|records| records.session_preferences(identity))?;
    let mut registry = RuntimeRegistry::first_party();
    let mut host = BindingSession::new(tmux).with_enter_delay(delay);
    // Both closures retain distinct driver outcome classes. The host performs
    // fresh endpoint and runtime verification, including after a NotSent result.
    let result = send_preferred(
        || match preferences.preferred_harness.as_ref() {
            None => ActionResult::Unsupported,
            Some(id) => match registry.send(id, &entry, message) {
                ActionResult::Unsupported => ActionResult::Unsupported,
                ActionResult::Completed(value) => ActionResult::Completed(value),
                ActionResult::Failed(error) => ActionResult::Failed(match error {
                    SendFailure::NotSent(_) => SendFailure::NotSent(Delivery::Unavailable),
                    SendFailure::Uncertain(_) => SendFailure::Uncertain(Delivery::Uncertain),
                    SendFailure::Denied(_) => SendFailure::Denied(Delivery::Unavailable),
                    SendFailure::AwaitingApproval(_) => {
                        SendFailure::AwaitingApproval(Delivery::Unavailable)
                    }
                }),
            },
        },
        || match host.send(&entry, message) {
            ActionResult::Unsupported => ActionResult::Unsupported,
            ActionResult::Completed(value) => ActionResult::Completed(value),
            ActionResult::Failed(error) => ActionResult::Failed(match error {
                SendFailure::NotSent(ActionError::Offline) => {
                    SendFailure::NotSent(Delivery::Offline)
                }
                SendFailure::NotSent(ActionError::Delivery(error)) => {
                    SendFailure::NotSent(Delivery::Transport(error))
                }
                SendFailure::Uncertain(ActionError::Delivery(error)) => {
                    SendFailure::Uncertain(Delivery::Transport(error))
                }
                SendFailure::NotSent(_) => SendFailure::NotSent(Delivery::Unavailable),
                SendFailure::Uncertain(_) => SendFailure::Uncertain(Delivery::Uncertain),
                SendFailure::Denied(_) => SendFailure::Denied(Delivery::Unavailable),
                SendFailure::AwaitingApproval(_) => {
                    SendFailure::AwaitingApproval(Delivery::Unavailable)
                }
            }),
        },
    );
    Ok(match result {
        ActionResult::Completed(_) => Delivery::Sent,
        ActionResult::Unsupported => Delivery::Unavailable,
        ActionResult::Failed(
            SendFailure::NotSent(value)
            | SendFailure::Uncertain(value)
            | SendFailure::Denied(value)
            | SendFailure::AwaitingApproval(value),
        ) => value,
    })
}

pub fn notify(storage: &mut Storage, hint: &OriginatorHint) -> WakeState {
    let recipient = hint
        .recipient_id
        .as_deref()
        .and_then(|id| current(storage, id).ok().flatten())
        .map(|entry| entry.identity.name)
        .unwrap_or_else(|| "recipient".into());
    let recipient: String = recipient
        .chars()
        .take(64)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let text = match hint.kind {
        HintKind::Reply => format!(
            "[tmt] reply from {recipient} to {}: tmt result {}",
            hint.request_id, hint.request_id
        ),
        HintKind::Timeout => format!(
            "[tmt] no reply yet from {recipient} to {} after {}s; still pending",
            hint.request_id,
            hint.timeout_ms as f64 / 1000.0
        ),
    };
    let outcome = match send(
        storage,
        &Tmux::default(),
        &hint.originator_id,
        &text,
        Duration::from_millis(500),
    ) {
        Ok(value) => value.wake_state(),
        Err(_) => WakeState::Unavailable,
    };
    // Failure to settle is an unknown outcome, never a reason to paste again.
    if RequestService::new(storage, tmt_adapters::request_runtime::wall_time_ms)
        .settle_hint(hint, outcome)
        .is_err()
    {
        return WakeState::Uncertain;
    }
    outcome
}

/// Unavailable process evidence is not proof that a blocking observer died.
/// This preparation is best-effort and cannot reject durable reply acceptance.
pub fn gone_waiter(
    storage: &mut Storage,
    request_id: &str,
) -> Option<tmt_core::request::notification::NotificationPolicy> {
    let Ok(Some(value)) =
        RequestService::new(&mut *storage, tmt_adapters::request_runtime::wall_time_ms)
            .notification(request_id)
    else {
        return None;
    };
    let Some(waiter) = &value.policy.waiter else {
        return None;
    };
    if observe_runtime_process(
        &tmt_adapters::process::UnixCommandRunner,
        waiter.pid(),
        Instant::now() + Duration::from_secs(1),
    )
    .is_ok_and(|observed| observed.matches(waiter) == RuntimeLiveness::Gone)
    {
        return Some(value.policy);
    }
    None
}
