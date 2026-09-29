use super::*;
use tmt_core::{
    binding::Binding,
    endpoint::{PaneObservation, ServerEvidence},
    identity::{Identity, Lifetime},
};

fn recipient() -> PaneIdentity {
    let server = ServerEvidence {
        host: tmt_core::host::HostKind::Tmux,
        server_id: "6c57fcc9-96b1-4022-b071-870b81288814".into(),
        socket_path: "/tmp/target-test.sock".into(),
        server_pid: 41,
        server_start_time: "1000".into(),
    };
    let identity = Identity {
        id: "identity-one".into(),
        name: "Alice".into(),
        canonical_name: "alice".into(),
        lifetime: Lifetime::Temporary,
        created_at: "created".into(),
        updated_at: "updated".into(),
    };
    let binding = Binding {
        id: "binding-one".into(),
        identity_id: identity.id.clone(),
        server: server.clone(),
        pane_id: "%14".into(),
        pane_pid: 42,
        session: Default::default(),
    };
    let pane = PaneObservation {
        id: binding.pane_id.clone(),
        target: Some("10.3".into()),
        cwd: None,
        command: "mock-agent".into(),
        pane_pid: binding.pane_pid,
        suggested_name: None,
        marker: Some(binding.marker(&identity)),
    };
    PaneIdentity {
        server,
        pane,
        identity: Some(identity),
        binding: Some(binding),
    }
}

fn snapshot(observed: &PaneIdentity) -> EndpointSnapshot {
    EndpointSnapshot {
        server: observed.server.clone(),
        panes: vec![observed.pane.clone()],
    }
}

#[test]
fn preserves_stable_pane_routing_when_layout_changes() {
    let observed = recipient();
    let mut fresh = snapshot(&observed);
    fresh.panes[0].target = Some("20.1".into());
    let endpoint = refreshed_endpoint(&observed, fresh).unwrap();
    assert_eq!(endpoint.server, observed.server);
    assert_eq!(endpoint.pane_id, "%14");
    assert_eq!(endpoint.pane_pid, 42);
}

#[test]
fn refuses_changed_bound_recipient_before_request_preparation() {
    let observed = recipient();
    let mutations: [fn(&mut EndpointSnapshot); 7] = [
        |s| s.server.socket_path = "/tmp/copied-server-uuid.sock".into(),
        |s| s.server.server_pid += 1,
        |s| s.server.server_start_time = "restarted".into(),
        |s| s.panes.clear(),
        |s| s.panes[0].pane_pid += 1,
        |s| s.panes[0].marker = None,
        |s| s.panes[0].marker.as_mut().unwrap().binding_id = "replacement".into(),
    ];
    for mutate in mutations {
        let mut fresh = snapshot(&observed);
        mutate(&mut fresh);
        let error = refreshed_endpoint(&observed, fresh).unwrap_err();
        assert_eq!(error.code, "RECONCILIATION_FAILED");
        assert_eq!(error.status, 1);
    }
}

#[test]
fn raw_pane_refresh_uses_fresh_process_evidence_but_never_invents_a_missing_pane() {
    let mut observed = recipient();
    observed.identity = None;
    observed.binding = None;
    let mut fresh = snapshot(&observed);
    fresh.panes[0].marker = None;
    fresh.panes[0].pane_pid = 99;
    assert_eq!(
        refreshed_endpoint(&observed, fresh.clone())
            .unwrap()
            .pane_pid,
        99
    );
    fresh.panes.clear();
    let error = refreshed_endpoint(&observed, fresh).unwrap_err();
    assert_eq!(error.code, "PANE_NOT_FOUND");
    assert_eq!(error.status, 3);
}
