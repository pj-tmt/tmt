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
        || foreground.provider_session.as_ref() != Some(&event.session)
        || ready.thread != event.session.as_str()
        || observe(&owner) != RuntimeLiveness::Alive
        || observe(&foreground.incarnation) != RuntimeLiveness::Alive
        || observe(&server) != RuntimeLiveness::Alive
    {
        return None;
    }
    if event.transition == SessionTransition::Started {
        return (current.state == tmt_core::binding::session::RuntimeState::Running)
            .then(|| current.clone());
    }
    current.transition(foreground, event.transition, None)
}
#[cfg(test)]
mod tests;
