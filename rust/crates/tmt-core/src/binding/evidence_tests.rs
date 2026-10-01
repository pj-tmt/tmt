use super::*;
use crate::{
    endpoint::{BindingMarker, EndpointSnapshot, PaneObservation, ServerEvidence},
    identity::Lifetime,
    names::validate_existing_name,
};

fn fixture(name: &str) -> (Identity, Binding, EndpointSnapshot) {
    let validated = validate_existing_name(name).expect("fixture identity name must be valid");
    let identity = Identity {
        id: "identity-1".into(),
        name: validated.display_name().into(),
        canonical_name: validated.canonical_name().into(),
        lifetime: Lifetime::Temporary,
        created_at: "created".into(),
        updated_at: "updated".into(),
    };
    let server = ServerEvidence {
        host: crate::host::HostKind::Tmux,
        server_id: "123e4567-e89b-42d3-a456-426614174000".into(),
        socket_path: "/tmp/tmt.sock".into(),
        server_pid: 321,
        server_start_time: "1700000000".into(),
    };
    let binding = Binding {
        id: "binding-1".into(),
        identity_id: identity.id.clone(),
        server: server.clone(),
        pane_id: "%9".into(),
        pane_pid: 654,
        pane_incarnation: None,
        session: Default::default(),
    };
    let pane = PaneObservation {
        id: binding.pane_id.clone(),
        target: Some("main:1.0".into()),
        cwd: Some("/repo".into()),
        command: "codex".into(),
        pane_pid: binding.pane_pid,
        pane_incarnation: None,
        suggested_name: Some("codex".into()),
        marker: Some(binding.marker(&identity)),
    };
    (
        identity,
        binding,
        EndpointSnapshot {
            server,
            panes: vec![pane],
        },
    )
}

fn entry(identity: &Identity, binding: &Binding) -> BindingEntry {
    BindingEntry {
        identity: identity.clone(),
        binding: Some(binding.clone()),
    }
}

fn live(snapshot: EndpointSnapshot) -> EndpointProbe {
    EndpointProbe::Live(snapshot)
}

#[test]
fn active_requires_agreement_between_server_pane_and_marker() {
    let (identity, binding, snapshot) = fixture("Alice");
    let expected = snapshot.panes[0].clone();

    assert_eq!(
        evaluate_binding(&entry(&identity, &binding), &live(snapshot)),
        BindingEvidence::Active(Box::new(expected))
    );
}

#[test]
fn a_tmux_binding_whose_name_now_reads_as_a_herdr_target_stays_active() {
    let (identity, binding, snapshot) = fixture("w1:p2");
    let expected = snapshot.panes[0].clone();
    assert_eq!(
        evaluate_binding(&entry(&identity, &binding), &live(snapshot)),
        BindingEvidence::Active(Box::new(expected))
    );
}

#[test]
fn presentation_fields_do_not_change_active_identity_evidence() {
    let (identity, binding, mut snapshot) = fixture("Alice");
    let pane = snapshot.panes.first_mut().expect("fixture pane");
    pane.target = Some("attached:7.3".into());
    pane.cwd = Some("/different/worktree".into());
    pane.command = "shell".into();
    pane.suggested_name = None;

    let expected = pane.clone();
    assert_eq!(
        evaluate_binding(&entry(&identity, &binding), &live(snapshot)),
        BindingEvidence::Active(Box::new(expected))
    );
}

#[test]
fn server_endpoint_changes_are_classified_without_guessing_death() {
    let (identity, binding, snapshot) = fixture("Alice");

    let mut socket_changed = snapshot.clone();
    socket_changed.server.socket_path = "/tmp/other.sock".into();
    assert_eq!(
        evaluate_binding(&entry(&identity, &binding), &live(socket_changed)),
        BindingEvidence::Unknown
    );

    let mut pid_changed = snapshot.clone();
    pid_changed.server.server_pid += 1;
    assert_eq!(
        evaluate_binding(&entry(&identity, &binding), &live(pid_changed)),
        BindingEvidence::EndpointLost
    );

    let mut start_changed = snapshot.clone();
    start_changed.server.server_start_time = "1700000001".into();
    assert_eq!(
        evaluate_binding(&entry(&identity, &binding), &live(start_changed)),
        BindingEvidence::EndpointLost
    );

    let mut id_changed = snapshot;
    id_changed.server.server_id = "123e4567-e89b-42d3-a456-426614174001".into();
    assert_eq!(
        evaluate_binding(&entry(&identity, &binding), &live(id_changed)),
        BindingEvidence::Unknown
    );
}

#[test]
fn pane_loss_and_replacement_are_endpoint_loss() {
    let (identity, binding, snapshot) = fixture("Alice");

    let mut missing = snapshot.clone();
    missing.panes.clear();
    assert_eq!(
        evaluate_binding(&entry(&identity, &binding), &live(missing)),
        BindingEvidence::EndpointLost
    );

    let mut replaced = snapshot;
    replaced.panes[0].pane_pid += 1;
    assert_eq!(
        evaluate_binding(&entry(&identity, &binding), &live(replaced)),
        BindingEvidence::EndpointLost
    );

    assert_eq!(
        evaluate_binding(&entry(&identity, &binding), &EndpointProbe::Dead),
        BindingEvidence::EndpointLost
    );
}

#[test]
fn a_reused_pane_pid_is_endpoint_loss_only_when_both_starts_are_known() {
    let (identity, mut binding, mut snapshot) = fixture("Alice");
    let evaluate = |binding: &Binding, snapshot: &EndpointSnapshot| {
        evaluate_binding(&entry(&identity, binding), &live(snapshot.clone()))
    };
    // Same pid, same start: the same pane.
    binding.pane_incarnation = Some("ps-v1:Thu Oct 1 09:00:00 2026".into());
    snapshot.panes[0].pane_incarnation = binding.pane_incarnation.clone();
    assert!(matches!(
        evaluate(&binding, &snapshot),
        BindingEvidence::Active(_)
    ));
    // Same pid, another start: the pid was reused, even with a copied marker.
    snapshot.panes[0].pane_incarnation = Some("ps-v1:Thu Oct 1 09:00:01 2026".into());
    assert_eq!(evaluate(&binding, &snapshot), BindingEvidence::EndpointLost);
    // An unknown start on either side proves nothing; the pid and marker decide.
    snapshot.panes[0].pane_incarnation = None;
    assert!(matches!(
        evaluate(&binding, &snapshot),
        BindingEvidence::Active(_)
    ));
    binding.pane_incarnation = None;
    snapshot.panes[0].pane_incarnation = Some("ps-v1:Thu Oct 1 09:00:01 2026".into());
    assert!(matches!(
        evaluate(&binding, &snapshot),
        BindingEvidence::Active(_)
    ));
}

#[test]
fn unavailable_observation_and_absent_binding_remain_unknown() {
    let (identity, binding, snapshot) = fixture("Alice");
    assert_eq!(
        evaluate_binding(&entry(&identity, &binding), &EndpointProbe::Unknown),
        BindingEvidence::Unknown
    );
    assert_eq!(
        evaluate_binding(
            &BindingEntry {
                identity,
                binding: None,
            },
            &live(snapshot),
        ),
        BindingEvidence::Unknown
    );
}

#[test]
fn marker_mismatches_never_become_endpoint_loss() {
    let (identity, binding, snapshot) = fixture("Alice");
    let cases = [
        ("missing marker", None),
        (
            "identity ID",
            Some(BindingMarker {
                identity_id: "other-identity".into(),
                ..binding.marker(&identity)
            }),
        ),
        (
            "binding ID",
            Some(BindingMarker {
                binding_id: "other-binding".into(),
                ..binding.marker(&identity)
            }),
        ),
        (
            "server ID",
            Some(BindingMarker {
                server_id: "123e4567-e89b-42d3-a456-426614174001".into(),
                ..binding.marker(&identity)
            }),
        ),
        (
            "pane PID",
            Some(BindingMarker {
                pane_pid: binding.pane_pid + 1,
                ..binding.marker(&identity)
            }),
        ),
    ];

    for (label, marker) in cases {
        let mut observed = snapshot.clone();
        observed.panes[0].marker = marker;
        assert_eq!(
            evaluate_binding(&entry(&identity, &binding), &live(observed)),
            BindingEvidence::MarkerMismatch,
            "{label}"
        );
    }

    let mut binding_identity_mismatch = binding.clone();
    binding_identity_mismatch.identity_id = "other-identity".into();
    assert_eq!(
        evaluate_binding(
            &entry(&identity, &binding_identity_mismatch),
            &live(snapshot.clone())
        ),
        BindingEvidence::MarkerMismatch
    );

    let mut identity_id_mismatch = identity.clone();
    identity_id_mismatch.id = "other-identity".into();
    assert_eq!(
        evaluate_binding(
            &entry(&identity_id_mismatch, &binding),
            &live(snapshot.clone())
        ),
        BindingEvidence::MarkerMismatch
    );

    let mut binding_id_mismatch = binding;
    binding_id_mismatch.id = "other-binding".into();
    assert_eq!(
        evaluate_binding(&entry(&identity, &binding_id_mismatch), &live(snapshot)),
        BindingEvidence::MarkerMismatch
    );
}

#[test]
fn marker_names_use_core_name_normalization_and_reject_malformed_values() {
    let (identity, binding, snapshot) = fixture("Alice");
    for name in [" ＡＬＩＣＥ ", "\u{feff}Ａlice\u{feff}"] {
        let mut observed = snapshot.clone();
        observed.panes[0].marker = Some(BindingMarker {
            name: name.into(),
            ..binding.marker(&identity)
        });
        let expected = observed.panes[0].clone();
        assert_eq!(
            evaluate_binding(&entry(&identity, &binding), &live(observed)),
            BindingEvidence::Active(Box::new(expected)),
            "{name:?}"
        );
    }

    let (nel_identity, nel_binding, nel_snapshot) = fixture("\u{85}Alice\u{85}");
    assert_eq!(
        evaluate_binding(
            &entry(&nel_identity, &nel_binding),
            &live(nel_snapshot.clone())
        ),
        BindingEvidence::Active(Box::new(nel_snapshot.panes[0].clone()))
    );

    for name in ["", "  ", "\u{feff}", "bad\u{1}name", "%1"] {
        let mut observed = snapshot.clone();
        observed.panes[0].marker = Some(BindingMarker {
            name: name.into(),
            ..binding.marker(&identity)
        });
        assert_eq!(
            evaluate_binding(&entry(&identity, &binding), &live(observed)),
            BindingEvidence::MarkerMismatch,
            "{name:?}"
        );
    }

    // A name inconsistent with its own canonical form is malformed.
    let mut canonical_mismatch = snapshot;
    canonical_mismatch.panes[0].marker = Some(BindingMarker {
        canonical_name: "other".into(),
        ..binding.marker(&identity)
    });
    assert_eq!(
        evaluate_binding(&entry(&identity, &binding), &live(canonical_mismatch)),
        BindingEvidence::MarkerMismatch
    );
}

#[test]
fn a_marker_still_carrying_the_name_before_a_rename_stays_active() {
    let (identity, binding, snapshot) = fixture("Alice");
    let mut observed = snapshot;
    observed.panes[0].marker = Some(BindingMarker {
        name: "Old Name".into(),
        canonical_name: "old name".into(),
        ..binding.marker(&identity)
    });
    let expected = observed.panes[0].clone();
    assert_eq!(
        evaluate_binding(&entry(&identity, &binding), &live(observed)),
        BindingEvidence::Active(Box::new(expected))
    );
}
