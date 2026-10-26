//! Which pane `tmt stop --close` closes, keeps or finds already gone, with a scripted host.
use super::*;
use tmt_core::{endpoint::PaneObservation, host::HostKind, identity::Lifetime};

/// A host that answers the probe as scripted and records every call it was asked to make.
struct Scripted {
    probe: EndpointProbe,
    opened: Option<bool>,
    close: Result<(), ActionError>,
    calls: Vec<&'static str>,
}
impl Scripted {
    fn new(probe: EndpointProbe) -> Self {
        Self {
            probe,
            opened: Some(true),
            close: Ok(()),
            calls: Vec::new(),
        }
    }
}
impl HostDriver for Scripted {
    fn begin_coordination(&mut self) {}
    fn budget_available(&self) -> bool {
        true
    }
    fn snapshot(&mut self, _: &[String]) -> Result<EndpointSnapshot, HostError> {
        unreachable!("close reads only the probe")
    }
    fn probe(&mut self, _: &ServerEvidence, _: &[String]) -> Result<EndpointProbe, HostError> {
        self.calls.push("probe");
        Ok(self.probe.clone())
    }
    fn publish(&mut self, _: &Binding, _: &Identity) -> Result<(), HostError> {
        unreachable!()
    }
    fn clear(&mut self, _: &Binding, _: &Identity) -> Result<bool, HostError> {
        unreachable!()
    }
    fn observed_runtime(&self, _: &Binding) -> Result<RuntimeState, CommandError> {
        Ok(RuntimeState::Unknown)
    }
    fn pane_incarnation(&mut self, _: u64) -> Result<Option<String>, HostError> {
        Ok(None)
    }
    fn input_activity(
        &mut self,
        _: &Binding,
    ) -> Result<tmt_core::driver::InputActivity, HostError> {
        Ok(tmt_core::driver::InputActivity::Unknown)
    }
    fn has_input(&self) -> bool {
        false
    }
    fn prompt(
        &mut self,
        _: &Binding,
        _: &str,
    ) -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>> {
        ActionResult::Unsupported
    }
    fn input(&mut self, _: &Binding, _: &str) -> Result<(), DeliveryError> {
        unreachable!()
    }
    fn focus_preflight(&self, _: Option<&Binding>) -> Result<(), ActionError> {
        Err(ActionError::HostUnsupported)
    }
    fn focus(&mut self, _: &Binding) -> Result<Focused, ActionError> {
        unreachable!()
    }
    fn close_preflight(&self) -> Result<(), ActionError> {
        Ok(())
    }
    fn opened_by_tmt(&mut self, _: &Binding) -> Result<Option<bool>, HostError> {
        self.calls.push("opened_by_tmt");
        Ok(self.opened)
    }
    fn close_pane(&mut self, _: &Binding) -> Result<(), ActionError> {
        self.calls.push("close_pane");
        std::mem::replace(&mut self.close, Ok(()))
    }
    fn name_agent(&mut self, _: &Binding, _: &str) -> Result<AgentNaming, HostError> {
        Ok(AgentNaming::NotLinked)
    }
}

fn entry() -> BindingEntry {
    BindingEntry {
        identity: Identity {
            id: "identity".into(),
            name: "Alice".into(),
            canonical_name: "alice".into(),
            lifetime: Lifetime::Saved,
            created_at: "created".into(),
            updated_at: "updated".into(),
        },
        binding: Some(Binding {
            id: "binding".into(),
            identity_id: "identity".into(),
            server: ServerEvidence {
                host: HostKind::pty(),
                server_id: "123e4567-e89b-42d3-a456-426614174000".into(),
                socket_path: "/tmp/tmt-pty/pty_1.sock".into(),
                server_pid: 321,
                server_start_time: "1700000000".into(),
            },
            pane_id: "pty_1".into(),
            pane_pid: 654,
            pane_incarnation: None,
            session: Default::default(),
        }),
    }
}

/// The probe of a live server whose one pane runs the binding's process and carries its marker.
fn live(entry: &BindingEntry) -> EndpointProbe {
    let binding = entry.binding.as_ref().unwrap();
    EndpointProbe::Live(EndpointSnapshot {
        server: binding.server.clone(),
        panes: vec![PaneObservation {
            id: binding.pane_id.clone(),
            target: None,
            cwd: None,
            command: "claude".into(),
            pane_pid: binding.pane_pid,
            pane_incarnation: None,
            suggested_name: None,
            marker: Some(binding.marker(&entry.identity)),
            location: None,
            agent_name: None,
        }],
    })
}

#[test]
fn a_present_pane_the_host_says_tmt_opened_is_closed_once() {
    let entry = entry();
    let mut host = Scripted::new(live(&entry));
    assert_eq!(
        close_opened_pane(&mut host, &entry).unwrap(),
        PaneClose::Closed
    );
    assert_eq!(host.calls, ["probe", "opened_by_tmt", "close_pane"]);
}

#[test]
fn a_pane_the_host_proves_gone_is_already_closed_and_nothing_else_is_asked() {
    let entry = entry();
    let mut host = Scripted::new(EndpointProbe::Dead);
    assert_eq!(
        close_opened_pane(&mut host, &entry).unwrap(),
        PaneClose::AlreadyGone
    );
    assert_eq!(
        host.calls,
        ["probe"],
        "a gone pane is neither marked nor closed"
    );
}

#[test]
fn an_unknown_presence_never_closes_and_is_not_taken_for_gone() {
    let entry = entry();
    let mut host = Scripted::new(EndpointProbe::Unknown);
    assert!(matches!(
        close_opened_pane(&mut host, &entry),
        Err(ActionError::Unverified)
    ));
    assert_eq!(host.calls, ["probe"]);
}

#[test]
fn an_unmarked_or_unsaid_mark_keeps_the_pane_and_never_asks_the_host_to_close() {
    for (opened, kept) in [(Some(false), Kept::NotOpenedByTmt), (None, Kept::Unknown)] {
        let entry = entry();
        let mut host = Scripted::new(live(&entry));
        host.opened = opened;
        assert_eq!(
            close_opened_pane(&mut host, &entry).unwrap(),
            PaneClose::Kept(kept)
        );
        assert_eq!(host.calls, ["probe", "opened_by_tmt"]);
    }
}

#[test]
fn a_refused_close_is_reported_once_and_not_retried() {
    let entry = entry();
    let mut host = Scripted::new(live(&entry));
    host.close = Err(ActionError::Unverified);
    assert!(matches!(
        close_opened_pane(&mut host, &entry),
        Err(ActionError::Unverified)
    ));
    assert_eq!(host.calls.iter().filter(|c| **c == "close_pane").count(), 1);
}
