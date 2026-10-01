//! Shared composition for requests and advisory hints. Drivers own IO policy.

use crate::{
    host::{ActionError, Host},
    process::{SupervisedProbeRunner, runtime::observe_runtime_process},
    runtime::{
        RuntimeError, RuntimeRegistry,
        channel::{ChannelFault, EvidenceError, PaneAddress},
    },
    storage::{Storage, StorageError},
};
use std::{
    path::{Path, PathBuf},
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
    /// (not ready, unreachable, its enrollment ended, or evidence about it that
    /// cannot be told). Nothing was sent and nothing was pasted: paste is only for
    /// sessions that never opted in. The evidence keeps the record at fault and the
    /// driver's recovery text for the output boundary to show.
    ChannelUnavailable(EvidenceError),
    Offline,
    Uncertain,
    Unavailable,
    Transport(crate::host::DeliveryError),
}

/// One delivery attempt: its outcome and what the attempt noticed on the way.
pub struct Attempt {
    pub delivery: Delivery,
    /// Channel records no driver could attribute to any pane, found while checking
    /// the pane before a baseline paste. They blocked nothing; the caller names them.
    pub unattributed: Vec<PathBuf>,
}

impl From<Delivery> for Attempt {
    fn from(delivery: Delivery) -> Self {
        Self {
            delivery,
            unattributed: Vec::new(),
        }
    }
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
            .map(|attempt| attempt.delivery.wake_state())
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
) -> Result<Attempt, StorageError> {
    match status(storage, identity)? {
        Availability::Ready => {}
        Availability::Offline => return Ok(Delivery::Offline.into()),
        Availability::Unavailable => return Ok(Delivery::Unavailable.into()),
    }
    let Some(entry) = current(storage, identity)? else {
        return Ok(Delivery::Offline.into());
    };
    let Some(binding) = entry.binding.as_ref() else {
        return Ok(Delivery::Offline.into());
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
        Err(fault) => return Ok(Delivery::ChannelUnavailable(fault.into()).into()),
    };
    let mut unattributed = Vec::new();
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
        || {
            // The baseline paste is the last resort for a session that never opted
            // in. A binding made after the pane's marker was lost has no record
            // under its own ID, so the drivers are asked whether an enrolled launch
            // still lives in this pane before anything is typed into it.
            let pane = PaneAddress {
                server: &binding.server,
                pane_id: &binding.pane_id,
                pane_pid: binding.pane_pid,
            };
            guarded_paste(
                &RuntimeRegistry::first_party(),
                &pane,
                &binding_id,
                directory.as_deref(),
                Instant::now() + PANE_EVIDENCE_BUDGET,
                &mut unattributed,
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
            )
        },
    );
    let delivery = match result {
        ActionResult::Completed(DeliveryAcceptance::Unacknowledged) => Delivery::Unacknowledged,
        ActionResult::Completed(_) => Delivery::Sent,
        ActionResult::Unsupported => Delivery::Unavailable,
        ActionResult::Failed(
            SendFailure::NotSent(value)
            | SendFailure::Uncertain(value)
            | SendFailure::Denied(value)
            | SendFailure::AwaitingApproval(value),
        ) => value,
    };
    Ok(Attempt {
        delivery,
        unattributed,
    })
}

type Outcome = ActionResult<DeliveryAcceptance, SendFailure<Delivery>>;

/// The one gate in front of a baseline paste (`send`); the raw-pane `talk` path
/// asks `pane_channel_evidence` directly. Runs `paste` only when no driver has
/// enrollment evidence for the pane; otherwise nothing is typed and the evidence,
/// with the record at fault and the driver's recovery text, becomes the outcome.
/// Records no driver could attribute are added to `unattributed` for the caller to
/// name. A channel directory that cannot be discovered is unknown, never "no
/// enrollment".
fn guarded_paste(
    registry: &RuntimeRegistry,
    pane: &PaneAddress<'_>,
    binding_id: &str,
    directory: Option<&Path>,
    deadline: Instant,
    unattributed: &mut Vec<PathBuf>,
    paste: impl FnOnce() -> Outcome,
) -> Outcome {
    let Some(directory) = directory else {
        return refused(ChannelFault::Unverifiable.into());
    };
    match pane_evidence(registry, pane, Some(binding_id), directory, deadline) {
        Ok(skipped) => {
            unattributed.extend(skipped);
            paste()
        }
        Err(evidence) => refused(evidence),
    }
}

fn refused(evidence: EvidenceError) -> Outcome {
    ActionResult::Failed(SendFailure::Denied(Delivery::ChannelUnavailable(evidence)))
}

/// A runtime driver's failure as a delivery outcome. Its class is kept, so
/// `send_preferred` never falls back after anything but `NotSent`, and a channel
/// that could not carry the request is reported as such rather than as a
/// generic unavailability.
fn runtime_failure(error: SendFailure<RuntimeError>) -> SendFailure<Delivery> {
    let unavailable = |error: RuntimeError| match error {
        RuntimeError::Channel(
            fault @ (ChannelFault::NotReady | ChannelFault::Unreachable | ChannelFault::Stale),
        ) => Delivery::ChannelUnavailable(fault.into()),
        _ => Delivery::Unavailable,
    };
    match error {
        SendFailure::NotSent(error) => SendFailure::NotSent(unavailable(error)),
        SendFailure::Uncertain(_) => SendFailure::Uncertain(Delivery::Uncertain),
        SendFailure::Denied(error) => SendFailure::Denied(unavailable(error)),
        SendFailure::AwaitingApproval(_) => SendFailure::AwaitingApproval(Delivery::Unavailable),
    }
}

/// Time one baseline delivery spends asking the drivers about the pane: directory
/// reads, record reads and a few exact process observations.
const PANE_EVIDENCE_BUDGET: Duration = Duration::from_secs(3);

/// Channel evidence for a pane about to be pasted to, read from the drivers' own
/// enrollment records through the pane address each persisted before its
/// foreground started. It never consults the stored binding: observation deletes
/// the binding of a pane that lost its marker and naming the pane again makes a new
/// one, so "no identity" or "no record under this binding" proves nothing about
/// whether a session in the pane opted in. `Ok` lets the baseline paste through and
/// returns the records no driver could attribute to any pane, for the caller to
/// name; `Err` is terminal: nothing may be pasted.
pub fn pane_channel_evidence(
    pane: &PaneAddress<'_>,
    binding_id: Option<&str>,
    directory: &Path,
) -> Result<Vec<PathBuf>, EvidenceError> {
    pane_evidence(
        &RuntimeRegistry::first_party(),
        pane,
        binding_id,
        directory,
        Instant::now() + PANE_EVIDENCE_BUDGET,
    )
}

fn pane_evidence(
    registry: &RuntimeRegistry,
    pane: &PaneAddress<'_>,
    binding_id: Option<&str>,
    directory: &Path,
    deadline: Instant,
) -> Result<Vec<PathBuf>, EvidenceError> {
    let evidence = registry.enrolled_in_pane(directory, pane, binding_id, deadline)?;
    if evidence.enrolled {
        return Err(ChannelFault::Inactive.into());
    }
    Ok(evidence.skipped)
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
        Ok(attempt) => attempt.delivery.wake_state(),
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

        fn enrolled_in_pane(
            &self,
            _: &Path,
            _: &PaneAddress<'_>,
            _: Option<&str>,
            _: Instant,
        ) -> Result<crate::runtime::channel::PaneEvidence, EvidenceError> {
            Ok(crate::runtime::channel::PaneEvidence::default())
        }
    }

    /// A runtime that claims nothing: the slot a fake channel is attached to on an
    /// otherwise empty registry, so these tests hold whichever first-party drivers
    /// register a channel.
    struct Slot;

    impl Driver for Slot {
        type Target = BindingEntry;
        type Error = RuntimeError;
        type Launch = crate::runtime::RuntimeCommand;
        fn claims(&self, _: &str) -> Option<HarnessId> {
            None
        }
    }

    fn fake_harness() -> HarnessId {
        HarnessId::new("fake-channel").unwrap()
    }

    fn registry_with(channel: Box<dyn crate::runtime::channel::RuntimeChannel>) -> RuntimeRegistry {
        let mut registry = RuntimeRegistry::default();
        registry
            .register(fake_harness(), "fake-channel", 0, Slot)
            .unwrap();
        registry.register_channel(&fake_harness(), channel).unwrap();
        registry
    }

    /// Answers every pane lookup the same way and records the binding it was asked
    /// about.
    struct InPane(
        Result<crate::runtime::channel::PaneEvidence, EvidenceError>,
        std::rc::Rc<std::cell::RefCell<Vec<Option<String>>>>,
    );

    impl crate::runtime::channel::RuntimeChannel for InPane {
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
            Ok(false)
        }

        fn enrolled_in_pane(
            &self,
            _: &Path,
            _: &PaneAddress<'_>,
            binding_id: Option<&str>,
            _: Instant,
        ) -> Result<crate::runtime::channel::PaneEvidence, EvidenceError> {
            self.1.borrow_mut().push(binding_id.map(str::to_owned));
            self.0.clone()
        }
    }

    #[test]
    fn a_pane_is_pasted_to_only_when_no_driver_has_enrollment_evidence_for_it() {
        use crate::runtime::channel::PaneEvidence;
        let server = tmt_core::endpoint::ServerEvidence {
            host: tmt_core::host::HostKind::Tmux,
            server_id: "server".into(),
            socket_path: "/tmp/tmux-test".into(),
            server_pid: 1,
            server_start_time: "start".into(),
        };
        let pane = PaneAddress {
            server: &server,
            pane_id: "%1",
            pane_pid: 4242,
        };
        let evidence = |registry: &RuntimeRegistry| {
            pane_evidence(
                registry,
                &pane,
                None,
                Path::new("/channels"),
                Instant::now() + Duration::from_secs(1),
            )
        };
        // The built-in drivers have no record in a directory that does not exist.
        assert_eq!(evidence(&RuntimeRegistry::first_party()), Ok(vec![]));
        // The answer comes from the drivers' own records, never from a stored
        // binding or the preferred harness: any driver's live or unconfirmed
        // enrollment blocks the paste, unattributable records are only reported,
        // and only "no enrollment" lets it through.
        let unreadable =
            EvidenceError::at(ChannelFault::InvalidRecord, Path::new("/channels/x.json"));
        let skipped = PathBuf::from("/channels/old.json");
        for (answer, expected) in [
            (Ok(PaneEvidence::default()), Ok(vec![])),
            (
                Ok(PaneEvidence {
                    enrolled: false,
                    skipped: vec![skipped.clone()],
                }),
                Ok(vec![skipped.clone()]),
            ),
            (
                Ok(PaneEvidence {
                    enrolled: true,
                    skipped: vec![],
                }),
                Err(EvidenceError::from(ChannelFault::Inactive)),
            ),
            (Err(unreadable.clone()), Err(unreadable.clone())),
        ] {
            let registry = registry_with(Box::new(InPane(answer.clone(), Default::default())));
            assert_eq!(evidence(&registry), expected, "{answer:?}");
        }
    }

    /// The identity path (a name, or a rebound pane) pastes only through
    /// `guarded_paste`: it must keep what the raw-pane path shows the user, the record
    /// and recovery of unknown evidence and the records it skipped.
    #[test]
    fn a_named_identity_is_refused_with_the_drivers_diagnostic_and_a_paste_reports_skipped_records()
    {
        use crate::runtime::channel::PaneEvidence;
        let server = tmt_core::endpoint::ServerEvidence {
            host: tmt_core::host::HostKind::Tmux,
            server_id: "server".into(),
            socket_path: "/tmp/tmux-test".into(),
            server_pid: 1,
            server_start_time: "start".into(),
        };
        let pane = PaneAddress {
            server: &server,
            pane_id: "%1",
            pane_pid: 4242,
        };
        let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let registry = |answer| registry_with(Box::new(InPane(answer, asked.clone())));
        let deadline = Instant::now() + Duration::from_secs(1);
        let directory = Some(Path::new("/channels"));
        let pasted = std::cell::Cell::new(false);
        let paste = || {
            pasted.set(true);
            ActionResult::Completed(DeliveryAcceptance::Submitted)
        };

        // Unknown evidence: nothing is pasted, and the outcome keeps the record at
        // fault and the driver's recovery text exactly as the driver gave them.
        let unknown = EvidenceError::at(
            ChannelFault::Unverifiable,
            Path::new("/channels/launch.json"),
        )
        .with_detail(
            "Remove it with: rm -- '/channels/launch.json' '/channels/launch.sock'.".into(),
        );
        let mut unattributed = Vec::new();
        let result = send_preferred(
            || ActionResult::Unsupported,
            || {
                guarded_paste(
                    &registry(Err(unknown.clone())),
                    &pane,
                    "binding",
                    directory,
                    deadline,
                    &mut unattributed,
                    || panic!("unknown evidence must not paste"),
                )
            },
        );
        match result {
            ActionResult::Failed(SendFailure::Denied(Delivery::ChannelUnavailable(evidence))) => {
                assert_eq!(evidence, unknown);
                assert!(evidence.message().contains("rm -- '/channels/launch.json'"));
            }
            _ => panic!("unknown evidence must end in a denied channel outcome"),
        }
        assert!(unattributed.is_empty());
        // The lookup was made for this binding's own record as well as its pane.
        assert_eq!(asked.borrow().as_slice(), [Some("binding".to_owned())]);

        // Records no driver could attribute block nothing: the paste happens and
        // they are handed back for the output boundary to name.
        let skipped = vec![PathBuf::from("/channels/old.json")];
        let result = guarded_paste(
            &registry(Ok(PaneEvidence {
                enrolled: false,
                skipped: skipped.clone(),
            })),
            &pane,
            "binding",
            directory,
            deadline,
            &mut unattributed,
            paste,
        );
        assert!(pasted.get() && matches!(result, ActionResult::Completed(_)));
        assert_eq!(unattributed, skipped);

        // An enrollment in the pane, and a channel directory that cannot be found,
        // are terminal too; neither pastes.
        pasted.set(false);
        for (answer, directory) in [
            (
                Ok(PaneEvidence {
                    enrolled: true,
                    skipped: vec![],
                }),
                directory,
            ),
            (Ok(PaneEvidence::default()), None),
        ] {
            let result = guarded_paste(
                &registry(answer),
                &pane,
                "binding",
                directory,
                deadline,
                &mut unattributed,
                paste,
            );
            assert!(matches!(
                result,
                ActionResult::Failed(SendFailure::Denied(Delivery::ChannelUnavailable(_)))
            ));
        }
        assert!(!pasted.get());
    }

    #[test]
    fn an_enrollment_decides_the_route_before_any_harness_preference_exists() {
        let claude = HarnessId::new("claude").unwrap();
        let fake = fake_harness();
        let route = |registry: &RuntimeRegistry,
                     directory: Option<&Path>,
                     preferred: Option<&HarnessId>| {
            route_harness(registry, directory, "binding", preferred.cloned())
        };
        let directory = Some(Path::new("/channels"));
        let enrolled = |answer| registry_with(Box::new(Evidence(answer)));
        // Paused after enroll and before admission: a record exists, no harness is
        // preferred yet (or an older one is), and the record's driver still owns it.
        let registry = enrolled(Ok(true));
        assert_eq!(route(&registry, directory, None), Ok(Some(fake.clone())));
        assert_eq!(
            route(&registry, directory, Some(&claude)),
            Ok(Some(fake.clone()))
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
        for preferred in [None, Some(&claude), Some(&fake)] {
            assert_eq!(
                route(&registry, directory, preferred),
                Err(ChannelFault::InvalidRecord)
            );
        }
        // An undiscoverable channel directory is unknown, not "never enrolled",
        // so it never recreates the baseline route, with or without a preference.
        for preferred in [None, Some(&claude), Some(&fake)] {
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
            Delivery::ChannelUnavailable(ChannelFault::NotReady.into()).wake_state(),
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
            SendFailure::Denied(Delivery::ChannelUnavailable(evidence)) if evidence.fault == ChannelFault::NotReady
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
