use std::fmt;

use tmt_core::{
    binding::{Binding, BindingEndpoint},
    endpoint::{EndpointProbe, EndpointSnapshot, PaneObservation, ServerEvidence},
    identity::Identity,
};

use super::super::test_support::pane;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProbeMode {
    Live,
    Dead,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EndpointFailure(pub(super) &'static str);

impl fmt::Display for EndpointFailure {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(self.0)
    }
}

impl std::error::Error for EndpointFailure {}

type BeforeClear = Box<dyn FnOnce(&mut FakeEndpoint) + Send>;

pub(super) struct FakeEndpoint {
    pub(super) server: ServerEvidence,
    panes: Vec<PaneObservation>,
    pub(super) probe_mode: ProbeMode,
    budget: bool,
    pub(super) expire_after_probe: Option<usize>,
    pub(super) expire_after_publish: bool,
    pub(super) hide_panes_after_current: Option<usize>,
    pub(super) replace_pane_after_current: Option<usize>,
    pub(super) publish_failure: bool,
    pub(super) publisher_liveness: tmt_core::binding::session::RuntimeLiveness,
    pub(super) publication_time_ms: u64,
    pub(super) after_publish: Option<Box<dyn FnOnce() + Send>>,
    pub(super) after_owner: Option<Box<dyn FnOnce() + Send>>,
    pub(super) after_probe: Option<Box<dyn FnOnce() + Send>>,
    pub(super) after_current: Option<Box<dyn FnOnce() + Send>>,
    pub(super) before_clear: Option<BeforeClear>,
    pub(super) late_marker_before_publish: Option<tmt_core::endpoint::BindingMarker>,
    pub(super) legacy_markers: std::collections::HashMap<String, String>,
    pub(super) replace_legacy_before_publish: Option<String>,
    pub(super) verification_failure: bool,
    pub(super) conditional_clear: bool,
    pub(super) clear_result: bool,
    pub(super) clear_failure: bool,
    pub(super) begin_calls: usize,
    pub(super) current_calls: usize,
    pub(super) publish_calls: usize,
    pub(super) clear_calls: usize,
    pub(super) probe_calls: Vec<(String, Vec<String>)>,
    /// What core's own process inspection would observe for each pane pid.
    pub(super) starts: std::collections::HashMap<u64, String>,
    pub(super) incarnation_failure: bool,
    pub(super) incarnation_calls: usize,
}

impl FakeEndpoint {
    pub(super) fn new(pane_ids: &[&str]) -> Self {
        Self {
            server: server("server-a", "/tmp/tmux-a.sock", 41, "start-a"),
            panes: pane_ids.iter().map(|id| pane(id, 100)).collect(),
            probe_mode: ProbeMode::Live,
            budget: true,
            expire_after_probe: None,
            expire_after_publish: false,
            hide_panes_after_current: None,
            replace_pane_after_current: None,
            publish_failure: false,
            publisher_liveness: tmt_core::binding::session::RuntimeLiveness::Alive,
            publication_time_ms: 1_000,
            after_publish: None,
            after_owner: None,
            after_probe: None,
            after_current: None,
            before_clear: None,
            late_marker_before_publish: None,
            legacy_markers: Default::default(),
            replace_legacy_before_publish: None,
            verification_failure: false,
            conditional_clear: true,
            clear_result: true,
            clear_failure: false,
            begin_calls: 0,
            current_calls: 0,
            publish_calls: 0,
            clear_calls: 0,
            probe_calls: Vec::new(),
            starts: Default::default(),
            incarnation_failure: false,
            incarnation_calls: 0,
        }
    }

    pub(super) fn pane_mut(&mut self, id: &str) -> &mut PaneObservation {
        self.panes
            .iter_mut()
            .find(|pane| pane.id == id)
            .expect("fixture pane exists")
    }

    fn snapshot(&self, server: &ServerEvidence, pane_ids: &[String]) -> EndpointSnapshot {
        let mut panes = self
            .panes
            .iter()
            .filter(|pane| pane_ids.iter().any(|id| id == &pane.id))
            .cloned()
            .collect::<Vec<_>>();
        if self.verification_failure && self.publish_calls > 0 {
            for pane in &mut panes {
                pane.marker = None;
            }
        }
        EndpointSnapshot {
            server: server.clone(),
            panes,
        }
    }
}

impl BindingEndpoint for FakeEndpoint {
    type Error = EndpointFailure;

    fn publication_owner(
        &mut self,
    ) -> Result<Option<tmt_core::endpoint::ProcessIncarnation>, Self::Error> {
        if let Some(callback) = self.after_owner.take() {
            callback();
        }
        Ok(Some(
            tmt_core::endpoint::ProcessIncarnation::new(42, "test-publisher").unwrap(),
        ))
    }
    fn publication_liveness(
        &mut self,
        _: &tmt_core::endpoint::ProcessIncarnation,
    ) -> Result<tmt_core::binding::session::RuntimeLiveness, Self::Error> {
        Ok(self.publisher_liveness)
    }
    fn publication_time_ms(&self) -> Option<u64> {
        Some(self.publication_time_ms)
    }

    fn current_host(&self) -> tmt_core::host::HostKind {
        self.server.host
    }

    fn pane_incarnation(
        &mut self,
        _: &ServerEvidence,
        pane_pid: u64,
    ) -> Result<Option<String>, Self::Error> {
        self.incarnation_calls += 1;
        if self.incarnation_failure {
            return Err(EndpointFailure("pane incarnation"));
        }
        Ok(self.starts.get(&pane_pid).cloned())
    }

    fn begin_coordination(&mut self) {
        self.begin_calls += 1;
    }

    fn budget_available(&self) -> bool {
        self.budget
            && !(self.expire_after_publish && self.publish_calls > 0)
            && self
                .expire_after_probe
                .is_none_or(|limit| self.probe_calls.len() < limit)
    }

    fn current_snapshot(&mut self, panes: &[String]) -> Result<EndpointSnapshot, Self::Error> {
        self.current_calls += 1;
        let mut snapshot = self.snapshot(&self.server, panes);
        if self
            .hide_panes_after_current
            .is_some_and(|call| self.current_calls >= call)
        {
            snapshot.panes.clear();
        }
        if self
            .replace_pane_after_current
            .is_some_and(|call| self.current_calls >= call)
        {
            for pane in &mut snapshot.panes {
                pane.pane_pid += 1;
            }
        }
        if let Some(callback) = self.after_current.take() {
            callback();
        }
        Ok(snapshot)
    }

    fn probe_binding(
        &mut self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, Self::Error> {
        self.probe_calls
            .push((server.server_id.clone(), panes.to_vec()));
        let result = match self.probe_mode {
            ProbeMode::Live => EndpointProbe::Live(self.snapshot(server, panes)),
            ProbeMode::Dead => EndpointProbe::Dead,
            ProbeMode::Unknown => EndpointProbe::Unknown,
        };
        if let Some(callback) = self.after_probe.take() {
            callback();
        }
        Ok(result)
    }

    fn publish(&mut self, binding: &Binding, identity: &Identity) -> Result<(), Self::Error> {
        if self.publish_failure {
            return Err(EndpointFailure("publish failed"));
        }
        self.publish_calls += 1;
        self.pane_mut(&binding.pane_id).marker = Some(binding.marker(identity));
        if let Some(after_publish) = self.after_publish.take() {
            after_publish();
        }
        Ok(())
    }

    fn legacy_marker(
        &mut self,
        _: &ServerEvidence,
        pane_id: &str,
    ) -> Result<Option<String>, Self::Error> {
        Ok(self.legacy_markers.get(pane_id).cloned())
    }

    fn publish_if_legacy(
        &mut self,
        binding: &Binding,
        identity: &Identity,
        expected_raw: &str,
    ) -> Result<bool, Self::Error> {
        if let Some(replacement) = self.replace_legacy_before_publish.take() {
            self.legacy_markers
                .insert(binding.pane_id.clone(), replacement);
        }
        if self
            .legacy_markers
            .get(&binding.pane_id)
            .map(String::as_str)
            != Some(expected_raw)
        {
            return Ok(false);
        }
        self.legacy_markers.remove(&binding.pane_id);
        self.publish(binding, identity).map(|()| true)
    }

    fn publish_if(
        &mut self,
        binding: &Binding,
        identity: &Identity,
        expected: Option<&str>,
    ) -> Result<bool, Self::Error> {
        if self.legacy_markers.contains_key(&binding.pane_id) {
            return Ok(false);
        }
        if let Some(late) = self.late_marker_before_publish.take() {
            self.pane_mut(&binding.pane_id).marker = Some(late);
        }
        let current = self
            .pane_mut(&binding.pane_id)
            .marker
            .as_ref()
            .map(|marker| marker.binding_id.clone());
        if current.as_deref() != expected && current.as_deref() != Some(binding.id.as_str()) {
            return Ok(false);
        }
        self.publish(binding, identity).map(|()| true)
    }

    fn conditional_clear(&mut self, _: &Binding) -> Result<bool, Self::Error> {
        Ok(self.conditional_clear)
    }

    fn clear(&mut self, binding: &Binding, _: &Identity) -> Result<bool, Self::Error> {
        self.clear_calls += 1;
        if let Some(callback) = self.before_clear.take() {
            callback(self);
        }
        if self.clear_failure {
            return Err(EndpointFailure("clear failed"));
        }
        if self.clear_result {
            let pane = self.pane_mut(&binding.pane_id);
            if pane
                .marker
                .as_ref()
                .is_some_and(|marker| marker.binding_id == binding.id)
            {
                pane.marker = None;
            }
        }
        Ok(self.clear_result)
    }
}

pub(super) fn server(id: &str, socket: &str, pid: u64, start: &str) -> ServerEvidence {
    ServerEvidence {
        host: tmt_core::host::HostKind::Tmux,
        server_id: id.into(),
        socket_path: socket.into(),
        server_pid: pid,
        server_start_time: start.into(),
    }
}
