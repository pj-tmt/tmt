//! Channel-only hook observations preserve the admitted foreground incarnation.
//! Environment is a locator, never authority; the private record supplies proof.
use super::{
    CodexObservation,
    channel_context::{BINDING_ENV, GENERATION_ENV},
    decode_hook,
    record::{Foreground, Record, Store},
};
use crate::{
    process::{UnixCommandRunner, runtime::observe_runtime_process},
    runtime::lifecycle::{HostEvidence, LifecycleObservation},
};
use std::time::{Duration, Instant};
use tmt_core::{
    binding::session::{
        BindingSessionState, DriverState, ProviderSessionId, RuntimeLiveness, SessionTransition,
    },
    endpoint::ProcessIncarnation,
};

pub fn decode(payload: &[u8]) -> Option<Box<dyn LifecycleObservation>> {
    let event = decode_hook(payload)?;
    let binding = std::env::var_os(BINDING_ENV);
    let generation = std::env::var_os(GENERATION_ENV);
    if binding.is_none() && generation.is_none() {
        return Some(Box::new(event));
    }
    // A malformed channel locator never enters the ordinary owned-resume path.
    let record = (|| {
        let binding = binding?.into_string().ok()?;
        let generation = generation?.into_string().ok()?;
        let directory = crate::config::ConfigPaths::discover()
            .ok()?
            .channel_directory();
        scoped_record(&directory, &binding, &generation)
    })();
    Some(Box::new(ChannelObservation { event, record }))
}
fn scoped_record(directory: &std::path::Path, binding: &str, generation: &str) -> Option<Record> {
    let record = Store::at(directory).read(binding).ok()??;
    (record.generation == generation).then_some(record)
}

pub fn activity_process(
    current: &BindingSessionState,
    observed: &ProcessIncarnation,
    session: &ProviderSessionId,
    host: HostEvidence,
    deadline: Instant,
) -> Option<ProcessIncarnation> {
    let binding = std::env::var_os(BINDING_ENV);
    let generation = std::env::var_os(GENERATION_ENV);
    if binding.is_none() && generation.is_none() {
        return Some(observed.clone());
    }
    let directory = crate::config::ConfigPaths::discover()
        .ok()?
        .channel_directory();
    let record = scoped_record(
        &directory,
        &binding?.into_string().ok()?,
        &generation?.into_string().ok()?,
    );
    activity_for_record(
        record.as_ref(),
        current,
        observed,
        session,
        host,
        deadline,
        &|p| {
            observe_runtime_process(&crate::process::SupervisedProbeRunner, p.pid(), deadline)
                .map(|value| value.matches(p))
                .unwrap_or(RuntimeLiveness::Unknown)
        },
    )
}

fn activity_for_record(
    record: Option<&Record>,
    current: &BindingSessionState,
    observed: &ProcessIncarnation,
    session: &ProviderSessionId,
    host: HostEvidence,
    deadline: Instant,
    observe: &impl Fn(&ProcessIncarnation) -> RuntimeLiveness,
) -> Option<ProcessIncarnation> {
    if !host.shared()
        || current.state != tmt_core::binding::session::RuntimeState::Running
        || Instant::now() >= deadline
    {
        return None;
    }
    let foreground = verified_foreground(record?, current, session, observed, observe)?;
    (Instant::now() < deadline).then_some(foreground)
}

pub fn wait_for_admission(
    payload: &[u8],
    deadline: Instant,
) -> Result<(), crate::runtime::lifecycle::LifecycleUnavailable> {
    use crate::runtime::lifecycle::LifecycleUnavailable;
    let (Some(binding), Some(generation)) = (
        std::env::var_os(BINDING_ENV),
        std::env::var_os(GENERATION_ENV),
    ) else {
        return Ok(());
    };
    let session = decode_hook(payload)
        .filter(|e| e.starting)
        .map(|e| e.session)
        .or_else(|| crate::runtime::hook_protocol::decode_prompt(payload))
        .or_else(|| super::decode_turn(payload).map(|e| e.session));
    let Some(session) = session else {
        return Ok(());
    };
    let binding = binding.into_string().map_err(|_| LifecycleUnavailable)?;
    let generation = generation.into_string().map_err(|_| LifecycleUnavailable)?;
    let directory = crate::config::ConfigPaths::discover()
        .map_err(|_| LifecycleUnavailable)?
        .channel_directory();
    let store = Store::at(&directory);
    wait_scoped(
        &generation,
        &session,
        deadline.min(Instant::now() + super::MAXIMUM_HOOK_ADMISSION_DURATION),
        || store.read(&binding).map_err(|_| LifecycleUnavailable),
        |remaining| std::thread::sleep(remaining.min(Duration::from_millis(10))),
    )
}

fn wait_scoped(
    generation: &str,
    session: &ProviderSessionId,
    deadline: Instant,
    mut read: impl FnMut() -> Result<Option<Record>, crate::runtime::lifecycle::LifecycleUnavailable>,
    mut wait: impl FnMut(Duration),
) -> Result<(), crate::runtime::lifecycle::LifecycleUnavailable> {
    use crate::runtime::lifecycle::LifecycleUnavailable;
    loop {
        let Some(record) = read()? else {
            return Ok(());
        };
        if record.generation != generation || record.ready.is_some() {
            return Ok(());
        }
        let Some(fresh) = record.fresh else {
            return Ok(());
        };
        if fresh
            .thread
            .as_deref()
            .is_some_and(|id| id != session.as_str())
        {
            return Ok(());
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or(LifecycleUnavailable)?;
        wait(remaining);
    }
}
struct ChannelObservation {
    event: CodexObservation,
    record: Option<Record>,
}
impl LifecycleObservation for ChannelObservation {
    fn session(&self) -> &ProviderSessionId {
        &self.event.session
    }
    fn starting(&self) -> bool {
        self.event.starting
    }
    fn verified_binding(&self) -> Option<&str> {
        let record = self.record.as_ref()?;
        let ready = record.ready.as_ref()?;
        (ready.thread == self.event.session.as_str()
            && matches!(record.foreground, Foreground::Known(_)))
        .then_some(record.binding_id.as_str())
    }
    fn driver_state(&self, previous: Option<&DriverState>, now_ms: u64) -> Option<DriverState> {
        self.event.driver_state(previous, now_ms)
    }
    fn propose(
        &self,
        current: &BindingSessionState,
        process: &ProcessIncarnation,
        _previous: RuntimeLiveness,
        host: HostEvidence,
        _owned_resume: bool,
    ) -> Option<BindingSessionState> {
        if !host.shared() {
            return None;
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        transition(&self.event, self.record.as_ref()?, current, process, &|p| {
            observe_runtime_process(&UnixCommandRunner, p.pid(), deadline)
                .map(|value| value.matches(p))
                .unwrap_or(RuntimeLiveness::Unknown)
        })
    }
}
fn transition(
    event: &CodexObservation,
    record: &Record,
    current: &BindingSessionState,
    process: &ProcessIncarnation,
    observe: &impl Fn(&ProcessIncarnation) -> RuntimeLiveness,
) -> Option<BindingSessionState> {
    verified_foreground(record, current, &event.session, process, observe)?;
    let foreground = current.key.as_ref()?;
    if event.transition == SessionTransition::Started {
        return (current.state == tmt_core::binding::session::RuntimeState::Running)
            .then(|| current.clone());
    }
    current.transition(foreground, event.transition, None)
}
/// The private channel record maps the verified server to the same admitted
/// foreground for lifecycle transitions and activity; it grants no admission.
fn verified_foreground(
    record: &Record,
    current: &BindingSessionState,
    session: &ProviderSessionId,
    process: &ProcessIncarnation,
    observe: &impl Fn(&ProcessIncarnation) -> RuntimeLiveness,
) -> Option<ProcessIncarnation> {
    let owner = record.launch_owner.incarnation()?;
    let ready = record.ready.as_ref()?;
    let server = ready.server.incarnation()?;
    let foreground = current.key.as_ref()?;
    let Foreground::Known(recorded) = &record.foreground else {
        return None;
    };
    if recorded.incarnation().as_ref() != Some(&foreground.incarnation) {
        return None;
    }
    if current.launch_owner.as_ref() != Some(&owner)
        || server != *process
        || foreground.incarnation == server
        || foreground.provider_session.as_ref() != Some(session)
        || ready.thread != session.as_str()
        || observe(&owner) != RuntimeLiveness::Alive
        || observe(&foreground.incarnation) != RuntimeLiveness::Alive
        || observe(&server) != RuntimeLiveness::Alive
    {
        return None;
    }
    Some(foreground.incarnation.clone())
}
#[cfg(test)]
mod tests;
