//! Shared composition for requests and advisory hints. Drivers own IO policy.

use crate::{
    host::{ActionError, Host},
    process::{SupervisedProbeRunner, runtime::observe_runtime_process},
    runtime::{RuntimeError, RuntimeRegistry, channel::ChannelFault},
    storage::{Storage, StorageError},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tmt_core::{
    binding::{
        BindingEntry, BindingRepository,
        session::{
            HarnessId, ObservedSessionKey, RuntimeLiveness, RuntimeState, SessionTransition,
        },
    },
    driver::{
        ActionResult, DeliveryAcceptance, Driver, InterfacePresence, InterfaceStatus, SendFailure,
        routing::send_preferred,
    },
    endpoint::ServerEvidence,
    request::{
        RequestService, WakeState,
        notification::{HintKind, OriginatorHint},
    },
};

pub enum Delivery {
    Sent,
    /// Written to a one-way channel with no provider receipt. The request is
    /// recorded as uncertain and its durable reply is still awaited, but it is
    /// never resent or pasted (see `contracts/claude-channel-v1.md`).
    Unacknowledged,
    /// The session opted into a channel that could not carry this request
    /// (not ready, unreachable, or its enrollment ended). Nothing was sent and
    /// nothing was pasted: paste is only for sessions that never opted in.
    ChannelUnavailable(ChannelFault),
    Offline,
    Uncertain,
    Unavailable,
    Transport(crate::host::DeliveryError),
}

impl Delivery {
    pub fn wake_state(&self) -> WakeState {
        match self {
            Self::Sent => WakeState::Sent,
            Self::Uncertain | Self::Unacknowledged => WakeState::Uncertain,
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

/// Claim once before attempting input. A lost settlement remains claimed and
/// must never cause a second paste; the durable request remains recoverable.
pub fn wake_request(
    storage: &mut Storage,
    request_id: &str,
    recipient_id: &str,
    notification: &str,
    delay: Duration,
) -> WakeState {
    use crate::request_runtime::wall_time_ms;
    let claim = match RequestService::new(storage, wall_time_ms).claim_wake(request_id) {
        Ok(claim) => claim,
        Err(_) => return WakeState::Claimed,
    };
    if !claim.claimed {
        return claim.state;
    }
    let eligible = RequestService::new(storage, wall_time_ms)
        .wake_recipient_is_eligible(request_id, recipient_id)
        .unwrap_or(false);
    let state = if eligible {
        send(storage, recipient_id, notification, delay)
            .map(|outcome| outcome.wake_state())
            .unwrap_or(WakeState::Unavailable)
    } else {
        WakeState::Unavailable
    };
    if RequestService::new(storage, wall_time_ms)
        .settle_wake(request_id, state)
        .is_err()
    {
        WakeState::Claimed
    } else {
        state
    }
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
        if let Ok(paths) = crate::config::ConfigPaths::discover() {
            let binding = entry.binding.as_ref().expect("verified binding");
            crate::pane_badge::refresh(
                &paths,
                &Host::for_server(&binding.server),
                binding,
                Instant::now() + Duration::from_secs(1),
            );
        }
    }
    Ok(())
}

/// Probes the identity's binding through the host that runs its server.
pub fn status(storage: &mut Storage, identity: &str) -> Result<Availability, StorageError> {
    let Some(mut entry) = current(storage, identity)? else {
        return Ok(Availability::Offline);
    };
    let Some(host) = entry
        .binding
        .as_ref()
        .map(|binding| Host::for_server(&binding.server))
    else {
        return Ok(Availability::Offline);
    };
    let mut session = host.session();
    match session.status(&entry) {
        ActionResult::Completed(InterfaceStatus {
            presence: InterfacePresence::Present,
            runtime,
        }) => {
            if runtime == RuntimeState::Ended {
                recover(storage, &RuntimeRegistry::first_party(), &mut entry)?;
            }
            Ok(match session.status(&entry) {
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
    identity: &str,
    message: &str,
    delay: Duration,
) -> Result<Delivery, StorageError> {
    match status(storage, identity)? {
        Availability::Ready => {}
        Availability::Offline => return Ok(Delivery::Offline),
        Availability::Unavailable => return Ok(Delivery::Unavailable),
    }
    let Some(entry) = current(storage, identity)? else {
        return Ok(Delivery::Offline);
    };
    let Some(binding) = entry.binding.as_ref() else {
        return Ok(Delivery::Offline);
    };
    let host = Host::for_server(&binding.server);
    let binding_id = binding.id.clone();
    let preferences =
        storage.with_binding_transaction(|records| records.session_preferences(identity))?;
    let mut registry = RuntimeRegistry::first_party();
    let directory = crate::config::ConfigPaths::discover()
        .ok()
        .map(|paths| paths.channel_directory());
    let harness = match route_harness(
        &registry,
        directory.as_deref(),
        &binding_id,
        preferences.preferred_harness,
    ) {
        Ok(harness) => harness,
        Err(fault) => return Ok(Delivery::ChannelUnavailable(fault)),
    };
    let mut session = host.session().with_enter_delay(delay);
    // Both closures retain distinct driver outcome classes. The host performs
    // fresh endpoint and runtime verification, including after a NotSent result.
    let result = send_preferred(
        || match harness.as_ref() {
            None => ActionResult::Unsupported,
            Some(id) => match registry.send(id, &entry, message) {
                ActionResult::Unsupported => ActionResult::Unsupported,
                ActionResult::Completed(value) => ActionResult::Completed(value),
                ActionResult::Failed(error) => ActionResult::Failed(runtime_failure(error)),
            },
        },
        || match session.send(&entry, message) {
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
        ActionResult::Completed(DeliveryAcceptance::Unacknowledged) => Delivery::Unacknowledged,
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

/// A runtime driver's failure as a delivery outcome. Its class is kept, so
/// `send_preferred` never falls back after anything but `NotSent`, and a channel
/// that could not carry the request is reported as such rather than as a
/// generic unavailability.
fn runtime_failure(error: SendFailure<RuntimeError>) -> SendFailure<Delivery> {
    let unavailable = |error: RuntimeError| match error {
        RuntimeError::Channel(
            fault @ (ChannelFault::NotReady | ChannelFault::Unreachable | ChannelFault::Stale),
        ) => Delivery::ChannelUnavailable(fault),
        _ => Delivery::Unavailable,
    };
    match error {
        SendFailure::NotSent(error) => SendFailure::NotSent(unavailable(error)),
        SendFailure::Uncertain(_) => SendFailure::Uncertain(Delivery::Uncertain),
        SendFailure::Denied(error) => SendFailure::Denied(unavailable(error)),
        SendFailure::AwaitingApproval(_) => SendFailure::AwaitingApproval(Delivery::Unavailable),
    }
}

/// Channel evidence for a pane that resolved to no active identity. Such a pane
/// is pasted to directly, but "no identity" does not prove it never opted in (a
/// binding that is not active resolves to none as well), so the pane's recorded
/// binding is asked whether its driver's channel has any enrollment on record.
/// `None` means there is none and the baseline transport applies; `Some` is a
/// terminal fault: nothing may be pasted.
pub fn pane_channel_evidence(
    storage: &mut Storage,
    server: &ServerEvidence,
    pane_id: &str,
    directory: &Path,
) -> Result<Option<ChannelFault>, StorageError> {
    let binding_id = storage.with_binding_transaction(|records| {
        Ok(records
            .entry_by_pane(server.host, pane_id, &server.server_id)?
            .and_then(|entry| entry.binding)
            .map(|binding| binding.id))
    })?;
    Ok(binding_id.and_then(|binding_id| {
        channel_evidence(&RuntimeRegistry::first_party(), directory, &binding_id)
    }))
}

fn channel_evidence(
    registry: &RuntimeRegistry,
    directory: &Path,
    binding_id: &str,
) -> Option<ChannelFault> {
    match registry.enrolled_harness(directory, binding_id) {
        Ok(None) => None,
        Ok(Some(_)) => Some(ChannelFault::Inactive),
        Err(fault) => Some(fault),
    }
}

/// The driver that carries a bound identity's delivery. An enrollment on record
/// names its driver before the identity prefers any harness (the preference is
/// written only after the launch is admitted), so it decides the route; the
/// preference only chooses among sessions that never opted in. Evidence that is
/// ambiguous or cannot be read is terminal, and so is a channel directory that
/// cannot be discovered: that is unknown, not proof that nothing is enrolled.
fn route_harness(
    registry: &RuntimeRegistry,
    directory: Option<&Path>,
    binding_id: &str,
    preferred: Option<HarnessId>,
) -> Result<Option<HarnessId>, ChannelFault> {
    let directory = directory.ok_or(ChannelFault::Unverifiable)?;
    Ok(registry
        .enrolled_harness(directory, binding_id)?
        .or(preferred))
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
        &hint.originator_id,
        &text,
        Duration::from_millis(500),
    ) {
        Ok(value) => value.wake_state(),
        Err(_) => WakeState::Unavailable,
    };
    // Failure to settle is an unknown outcome, never a reason to paste again.
    if RequestService::new(storage, crate::request_runtime::wall_time_ms)
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
    let Ok(Some(value)) = RequestService::new(&mut *storage, crate::request_runtime::wall_time_ms)
        .notification(request_id)
    else {
        return None;
    };
    let Some(waiter) = &value.policy.waiter else {
        return None;
    };
    if observe_runtime_process(
        &crate::process::UnixCommandRunner,
        waiter.pid(),
        Instant::now() + Duration::from_secs(1),
    )
    .is_ok_and(|observed| observed.matches(waiter) == RuntimeLiveness::Gone)
    {
        return Some(value.policy);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Evidence(Result<bool, ChannelFault>);

    impl crate::runtime::channel::RuntimeChannel for Evidence {
        fn preflight(
            &self,
            _: &std::ffi::OsStr,
            _: &Path,
            _: Instant,
        ) -> Result<Option<String>, crate::runtime::channel::ChannelError> {
            Ok(None)
        }

        fn enroll(
            &self,
            _: &crate::runtime::channel::ChannelPlan<'_>,
        ) -> Result<
            Box<dyn crate::runtime::channel::ChannelEnrollment>,
            crate::runtime::channel::ChannelError,
        > {
            Err(crate::runtime::channel::ChannelError::Unsupported)
        }

        fn enrolled(&self, _: &Path, _: &str) -> Result<bool, ChannelFault> {
            self.0
        }
    }

    #[test]
    fn a_pane_without_identity_pastes_only_when_no_driver_has_channel_evidence() {
        let codex = HarnessId::new("codex").unwrap();
        let evidence = |registry: &RuntimeRegistry| {
            channel_evidence(registry, Path::new("/channels"), "binding")
        };
        // The built-in drivers have no record in a directory that does not exist.
        assert_eq!(evidence(&RuntimeRegistry::first_party()), None);
        // The answer depends on the binding's record alone, never on which harness
        // the identity currently prefers: any driver's evidence blocks the paste,
        // and only "no enrollment" lets it through.
        for (answer, expected) in [
            (Ok(false), None),
            (Ok(true), Some(ChannelFault::Inactive)),
            (
                Err(ChannelFault::InvalidRecord),
                Some(ChannelFault::InvalidRecord),
            ),
        ] {
            let mut registry = RuntimeRegistry::first_party();
            registry
                .register_channel(&codex, Box::new(Evidence(answer)))
                .unwrap();
            assert_eq!(evidence(&registry), expected, "{answer:?}");
        }
    }

    #[test]
    fn an_enrollment_decides_the_route_before_any_harness_preference_exists() {
        let claude = HarnessId::new("claude").unwrap();
        let codex = HarnessId::new("codex").unwrap();
        let route = |registry: &RuntimeRegistry,
                     directory: Option<&Path>,
                     preferred: Option<&HarnessId>| {
            route_harness(registry, directory, "binding", preferred.cloned())
        };
        let directory = Some(Path::new("/channels"));
        let enrolled = |answer| {
            let mut registry = RuntimeRegistry::first_party();
            registry
                .register_channel(&codex, Box::new(Evidence(answer)))
                .unwrap();
            registry
        };
        // Paused after enroll and before admission: a record exists, no harness is
        // preferred yet (or an older one is), and the record's driver still owns it.
        let registry = enrolled(Ok(true));
        assert_eq!(route(&registry, directory, None), Ok(Some(codex.clone())));
        assert_eq!(
            route(&registry, directory, Some(&claude)),
            Ok(Some(codex.clone()))
        );
        // No enrollment: the preference alone chooses, and none means no driver.
        let registry = enrolled(Ok(false));
        assert_eq!(route(&registry, directory, None), Ok(None));
        assert_eq!(
            route(&registry, directory, Some(&claude)),
            Ok(Some(claude.clone()))
        );
        // Unreadable evidence is terminal whatever is preferred.
        let registry = enrolled(Err(ChannelFault::InvalidRecord));
        for preferred in [None, Some(&claude), Some(&codex)] {
            assert_eq!(
                route(&registry, directory, preferred),
                Err(ChannelFault::InvalidRecord)
            );
        }
        // An undiscoverable channel directory is unknown, not "never enrolled",
        // so it never recreates the baseline route, with or without a preference.
        for preferred in [None, Some(&claude), Some(&codex)] {
            assert_eq!(
                route(&enrolled(Ok(false)), None, preferred),
                Err(ChannelFault::Unverifiable)
            );
        }
    }

    #[test]
    fn an_unacknowledged_channel_write_is_an_uncertain_wake_that_is_never_offline_or_unavailable() {
        assert_eq!(Delivery::Unacknowledged.wake_state(), WakeState::Uncertain);
        assert_eq!(Delivery::Sent.wake_state(), WakeState::Sent);
        assert_eq!(Delivery::Unavailable.wake_state(), WakeState::Unavailable);
        assert_eq!(
            Delivery::ChannelUnavailable(ChannelFault::NotReady).wake_state(),
            WakeState::Unavailable
        );
    }

    /// An opted-in session's channel outcomes must never reach the paste
    /// fallback, whichever way the routing policy is composed.
    #[test]
    fn an_opted_in_channel_that_cannot_carry_the_request_never_falls_back_to_paste() {
        for fault in [
            ChannelFault::NotReady,
            ChannelFault::Unreachable,
            ChannelFault::Stale,
            ChannelFault::Mismatch,
            ChannelFault::InvalidRecord,
            ChannelFault::Unverifiable,
            ChannelFault::Refused,
            ChannelFault::TooLarge,
            ChannelFault::Uncertain,
        ] {
            let result = send_preferred(
                || {
                    ActionResult::Failed(runtime_failure(SendFailure::Denied(
                        RuntimeError::Channel(fault),
                    )))
                },
                || panic!("{fault:?} must not paste"),
            );
            assert!(matches!(
                result,
                ActionResult::Failed(SendFailure::Denied(_))
            ));
        }
        assert!(matches!(
            runtime_failure(SendFailure::Denied(RuntimeError::Channel(
                ChannelFault::NotReady
            ))),
            SendFailure::Denied(Delivery::ChannelUnavailable(ChannelFault::NotReady))
        ));
        assert!(matches!(
            runtime_failure(SendFailure::Denied(RuntimeError::Channel(
                ChannelFault::Refused
            ))),
            SendFailure::Denied(Delivery::Unavailable)
        ));
        assert!(matches!(
            runtime_failure(SendFailure::Uncertain(RuntimeError::Channel(
                ChannelFault::Uncertain
            ))),
            SendFailure::Uncertain(Delivery::Uncertain)
        ));
    }
}
