//! Declarative hook decoding uses core's existing session and opaque state policy.
use crate::runtime::{
    driver_state,
    lifecycle::{
        self, HostEvidence, LifecycleObservation, LifecycleUnavailable, RuntimeLifecycle, TurnEnd,
    },
};
use std::time::Instant;
use tmt_core::{
    binding::session::{
        BindingSessionState, DriverState, ProviderSessionId, RuntimeLiveness, RuntimeMode,
        SessionTransition,
        activity::{ActivityPhase, Event},
    },
    endpoint::ProcessIncarnation,
};
use tmt_driver_protocol::{HookEffect, HookObservation, RuntimeDeclaration, Transition};

pub(super) struct DeclaredLifecycle(pub RuntimeDeclaration);
struct Observation {
    session: ProviderSessionId,
    model: Option<String>,
    transition: SessionTransition,
    starting: bool,
}
impl LifecycleObservation for Observation {
    fn session(&self) -> &ProviderSessionId {
        &self.session
    }
    fn starting(&self) -> bool {
        self.starting
    }
    fn driver_state(&self, previous: Option<&DriverState>, _: u64) -> Option<DriverState> {
        driver_state::after_start(self.model.as_deref(), None, previous)
    }
    fn propose(
        &self,
        current: &BindingSessionState,
        process: &ProcessIncarnation,
        previous: RuntimeLiveness,
        _: HostEvidence,
        _: bool,
    ) -> Option<BindingSessionState> {
        lifecycle::propose_session(
            &self.session,
            self.transition,
            self.starting,
            current,
            process,
            previous,
        )
    }
}
impl DeclaredLifecycle {
    fn read(&self, payload: &[u8]) -> Option<HookObservation> {
        self.0.decode_hook(payload)
    }
}
impl RuntimeLifecycle for DeclaredLifecycle {
    fn decode(&self, payload: &[u8]) -> Option<Box<dyn LifecycleObservation>> {
        let hook = self.read(payload)?;
        if !matches!(hook.effect, HookEffect::Start | HookEffect::End) {
            return None;
        }
        let transition = match hook.transition? {
            Transition::Started => SessionTransition::Started,
            Transition::Resumed => SessionTransition::Resumed,
            Transition::Cleared => SessionTransition::Cleared,
            Transition::Compacted => SessionTransition::Compacted,
            Transition::Forked => SessionTransition::Forked,
            Transition::Ended => SessionTransition::Ended,
        };
        Some(Box::new(Observation {
            session: ProviderSessionId::new(&hook.session).ok()?,
            model: hook.model,
            transition,
            starting: hook.effect == HookEffect::Start,
        }))
    }
    fn decode_prompt(&self, payload: &[u8]) -> Option<ProviderSessionId> {
        let hook = self.read(payload)?;
        (hook.effect == HookEffect::Working)
            .then(|| ProviderSessionId::new(&hook.session).ok())
            .flatten()
    }
    fn decode_activity(&self, payload: &[u8]) -> Option<Event> {
        let hook = self.read(payload)?;
        let phase = match hook.effect {
            HookEffect::Working => ActivityPhase::Working,
            HookEffect::Idle => ActivityPhase::Idle,
            _ => return None,
        };
        Some(Event {
            phase,
            turn: hook
                .turn
                .as_deref()
                .map(ProviderSessionId::new)
                .transpose()
                .ok()?,
        })
    }
    fn decode_turn(&self, payload: &[u8]) -> Option<TurnEnd> {
        let hook = self.read(payload)?;
        (hook.effect == HookEffect::Idle)
            .then(|| {
                Some(TurnEnd {
                    session: ProviderSessionId::new(&hook.session).ok()?,
                    transcript: hook.transcript.map(Into::into),
                })
            })
            .flatten()
    }
    fn reads_state(&self, version: u16) -> bool {
        driver_state::reads(version)
    }
    fn state_model(&self, state: &DriverState) -> Option<String> {
        driver_state::state_model(state)
    }
    fn state_usage(&self, state: &DriverState) -> Option<driver_state::Usage> {
        driver_state::state_usage(state)
    }
    fn host_evidence(&self) -> Result<HostEvidence, LifecycleUnavailable> {
        Ok(HostEvidence::Independent { runtime_pid: None })
    }
    fn mode(&self, _: HostEvidence) -> Option<RuntimeMode> {
        RuntimeMode::new("default").ok()
    }
    fn observe_in_pane(
        &self,
        caller: u64,
        pane: u64,
        deadline: Instant,
    ) -> Option<ProcessIncarnation> {
        crate::runtime::evidence::observe_named_in_pane(
            &crate::process::SupervisedProbeRunner,
            caller,
            pane,
            deadline,
            &self
                .0
                .executables()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )
    }
}
