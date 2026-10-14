use super::*;

fn member(id: &str, name: &str) -> OpsMember {
    OpsMember {
        id: id.into(),
        name: name.into(),
        state: None,
        waiting: false,
        presence: None,
        usage: None,
    }
}

fn roster() -> OpsRoster {
    OpsRoster {
        members: [
            member("lead", "Lead"),
            member("one", "Alice"),
            member("free", "Free"),
        ]
        .into_iter()
        .map(|member| (member.id.clone(), member))
        .collect(),
        squads: vec![OpsSquad {
            room_id: "room".into(),
            name: "product".into(),
            members: vec!["lead".into(), "one".into()],
            lead: Some("lead".into()),
            member_states: BTreeMap::new(),
            observation: OpsObservation::Complete,
        }],
        membership_complete: true,
        observation: OpsObservation::Complete,
    }
}

fn target(room: Option<&str>, member: &str) -> OpsTarget {
    OpsTarget::Member {
        room_id: room.map(str::to_owned),
        identity_id: member.into(),
    }
}

#[test]
fn virtual_pool_is_first_and_contains_only_zero_membership_identities() {
    let roster = roster();
    let mut workspace = OpsWorkspace {
        pool_expanded: true,
        ..Default::default()
    };
    let mut preferences = OpsPreferences::default();
    workspace.selected = Some(OpsTarget::Pool);
    assert!(!workspace.toggle_pin(&mut preferences));
    let rows = workspace.rows(&roster, &preferences);
    assert_eq!(rows[0].target, OpsTarget::Pool);
    assert_eq!(rows[0].count, Some(1));
    assert_eq!(rows[1].target, target(None, "free"));
    assert_eq!(rows[2].target, OpsTarget::Squad("room".into()));
    assert_eq!(rows[2].count, Some(2)); // Includes the lead exactly once.
    assert!(!rows.iter().any(|row| row.target == target(None, "one")));
}

#[test]
fn partial_or_stale_membership_never_proves_an_identity_is_unassigned() {
    for observation in [
        OpsObservation::Partial,
        OpsObservation::Stale,
        OpsObservation::Unavailable,
    ] {
        let mut roster = roster();
        roster.observation = observation;
        roster.membership_complete = false;
        roster.squads[0].observation = observation;
        let rows = OpsWorkspace {
            pool_expanded: true,
            ..Default::default()
        }
        .rows(&roster, &OpsPreferences::default());
        assert_eq!(rows[0].count, None);
        assert_eq!(rows[0].observation, observation);
        assert_eq!(rows[1].target, OpsTarget::Squad("room".into()));
        assert_eq!(rows[1].count, None);
        assert!(
            !rows
                .iter()
                .any(|row| matches!(row.target, OpsTarget::Member { room_id: None, .. }))
        );
    }
}

#[test]
fn a_failed_squad_source_prevents_a_false_known_zero_pool() {
    let mut roster = roster();
    roster.squads[0].observation = OpsObservation::Unavailable;
    let rows = OpsWorkspace::default().rows(&roster, &OpsPreferences::default());
    assert_eq!(rows[0].count, None);
    assert_eq!(rows[0].observation, OpsObservation::Partial);
}

#[test]
fn known_empty_is_distinct_from_unavailable_and_offline_is_not_missing() {
    let complete = OpsRoster {
        observation: OpsObservation::Complete,
        membership_complete: true,
        ..Default::default()
    };
    let rows = OpsWorkspace::default().rows(&complete, &OpsPreferences::default());
    assert_eq!(rows[0].count, Some(0));
    let rows = OpsWorkspace::default().rows(&OpsRoster::default(), &OpsPreferences::default());
    assert_eq!(rows[0].count, None);
    assert_eq!(rows[0].observation, OpsObservation::Unavailable);
    let mut roster = roster();
    roster.members.get_mut("free").unwrap().presence = Some(false);
    let rows = OpsWorkspace {
        pool_expanded: true,
        ..Default::default()
    }
    .rows(&roster, &OpsPreferences::default());
    assert_eq!(rows[1].presence, Some(false));
    assert_eq!(rows[1].usage, None);
    roster.members.get_mut("free").unwrap().usage = Some(0);
    assert_eq!(
        OpsWorkspace {
            pool_expanded: true,
            ..Default::default()
        }
        .rows(&roster, &OpsPreferences::default())[1]
            .usage,
        Some(0)
    );
}

#[test]
fn collapse_retains_child_attention_without_making_a_second_member_count() {
    let mut roster = roster();
    roster.squads[0].member_states.insert(
        "one".into(),
        OpsMembershipState {
            state: Some("blocked".into()),
            waiting: false,
            rank: None,
        },
    );
    roster.squads[0].members.push("one".into());
    let rows = OpsWorkspace::default().rows(&roster, &OpsPreferences::default());
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].state.as_deref(), Some("blocked"));
    assert_eq!(rows[1].count, Some(2));
    roster.squads[0].member_states.insert(
        "lead".into(),
        OpsMembershipState {
            state: None,
            waiting: true,
            rank: None,
        },
    );
    assert_eq!(
        OpsWorkspace::default().rows(&roster, &OpsPreferences::default())[1]
            .state
            .as_deref(),
        Some("waiting")
    );
}

#[test]
fn selection_stays_on_the_same_uuid_when_filtered_collapsed_or_missing() {
    let mut roster = roster();
    let selected = target(Some("room"), "one");
    let mut workspace = OpsWorkspace {
        selected: Some(selected.clone()),
        ..Default::default()
    };
    let preferences = OpsPreferences::default();
    let rows = workspace.rows(&roster, &preferences);
    assert_eq!(
        workspace.selection_state(&roster, &rows),
        SelectionState::Hidden
    );
    assert!(workspace.toggle_expanded(&OpsTarget::Squad("room".into())));
    let rows = workspace.rows(&roster, &preferences);
    assert_eq!(
        workspace.selection_state(&roster, &rows),
        SelectionState::Visible
    );
    workspace.query = "absent".into();
    let rows = workspace.rows(&roster, &preferences);
    assert_eq!(
        workspace.selection_state(&roster, &rows),
        SelectionState::Hidden
    );
    roster.squads[0].members.retain(|id| id != "one");
    assert_eq!(
        workspace.selection_state(&roster, &rows),
        SelectionState::Missing
    );
    roster.membership_complete = false;
    assert_eq!(
        workspace.selection_state(&roster, &rows),
        SelectionState::Unknown
    );
    assert_eq!(workspace.selected, Some(selected)); // Never falls through to another member.
}

#[test]
fn renamed_names_and_membership_in_another_squad_keep_distinct_anchors() {
    let mut roster = roster();
    roster.squads.push(OpsSquad {
        room_id: "other".into(),
        name: "other".into(),
        ..roster.squads[0].clone()
    });
    let mut workspace = OpsWorkspace {
        selected: Some(target(Some("room"), "one")),
        ..Default::default()
    };
    workspace.expanded.extend(["room".into(), "other".into()]);
    roster.members.get_mut("one").unwrap().name = "Renamed".into();
    let rows = workspace.rows(&roster, &OpsPreferences::default());
    assert_eq!(rows.iter().filter(|row| row.title == "Renamed").count(), 2);
    assert_eq!(
        workspace.selection_state(&roster, &rows),
        SelectionState::Visible
    );
    roster.squads[0].members.retain(|id| id != "one");
    let rows = workspace.rows(&roster, &OpsPreferences::default());
    assert_eq!(
        workspace.selection_state(&roster, &rows),
        SelectionState::Missing
    );
    assert!(
        rows.iter()
            .any(|row| row.target == target(Some("other"), "one"))
    );
    assert!(!rows.iter().any(|row| row.target == target(None, "one")));
}

#[test]
fn pinned_squads_stay_before_unpinned_in_both_sort_directions() {
    let mut roster = roster();
    roster.squads.push(OpsSquad {
        room_id: "pinned".into(),
        name: "z".into(),
        members: vec![],
        lead: None,
        member_states: BTreeMap::new(),
        observation: OpsObservation::Complete,
    });
    let workspace = OpsWorkspace {
        selected: Some(OpsTarget::Squad("pinned".into())),
        ..Default::default()
    };
    let mut preferences = OpsPreferences::default();
    assert!(workspace.toggle_pin(&mut preferences));
    for sort in [OpsSort::Name, OpsSort::Attention, OpsSort::Members] {
        for reversed in [false, true] {
            preferences.sort = sort;
            preferences.reversed = reversed;
            let rows = workspace.rows(&roster, &preferences);
            assert_eq!(rows[0].target, OpsTarget::Pool);
            assert_eq!(rows[1].target, OpsTarget::Squad("pinned".into()));
            assert_eq!(
                workspace.selection_state(&roster, &rows),
                SelectionState::Visible
            );
        }
    }
    assert!(workspace.toggle_pin(&mut preferences));
    assert!(preferences.pinned.is_empty());
}

#[test]
fn member_and_virtual_pool_cannot_mutate_pin_preferences_or_expand() {
    let mut workspace = OpsWorkspace {
        selected: Some(target(None, "free")),
        ..Default::default()
    };
    let mut preferences = OpsPreferences::default();
    assert!(!workspace.toggle_pin(&mut preferences));
    assert!(!workspace.toggle_expanded(&target(None, "free")));
    workspace.selected = None;
    assert!(!workspace.toggle_pin(&mut preferences));
    assert_eq!(
        workspace.selection_state(&roster(), &[]),
        SelectionState::None
    );
}

#[test]
fn duplicate_names_sort_by_uuid_and_lead_is_first_among_unique_members() {
    let mut roster = roster();
    roster.squads.push(OpsSquad {
        room_id: "aardvark".into(),
        ..roster.squads[0].clone()
    });
    let mut workspace = OpsWorkspace::default();
    workspace.expanded.insert("room".into());
    let rows = workspace.rows(&roster, &OpsPreferences::default());
    assert_eq!(rows[1].target, OpsTarget::Squad("aardvark".into()));
    assert_eq!(rows[3].target, target(Some("room"), "lead"));
    assert!(rows[3].lead);
    assert!(!rows[4].lead);
    assert_eq!(OpsSort::Name.next().next().next(), OpsSort::Name);
}

#[test]
fn dense_roster_preserves_selected_target_across_reversal_and_search() {
    let mut roster = OpsRoster {
        observation: OpsObservation::Complete,
        membership_complete: true,
        ..Default::default()
    };
    for i in 0..150 {
        let id = format!("identity-{i:03}");
        roster
            .members
            .insert(id.clone(), member(&id, &format!("Member {i:03}")));
    }
    for i in 0..36 {
        roster.squads.push(OpsSquad {
            room_id: format!("room-{i:02}"),
            name: format!("squad-{i:02}"),
            members: vec![format!("identity-{i:03}")],
            lead: None,
            member_states: BTreeMap::new(),
            observation: OpsObservation::Complete,
        });
    }
    let selected = target(Some("room-20"), "identity-020");
    let mut workspace = OpsWorkspace {
        selected: Some(selected.clone()),
        pool_expanded: true,
        ..Default::default()
    };
    workspace
        .expanded
        .extend(roster.squads.iter().map(|squad| squad.room_id.clone()));
    let mut preferences = OpsPreferences::default();
    for reversed in [false, true] {
        preferences.reversed = reversed;
        let rows = workspace.rows(&roster, &preferences);
        assert_eq!(rows.len(), 1 + 114 + 36 + 36);
        assert_eq!(
            workspace.selection_state(&roster, &rows),
            SelectionState::Visible
        );
    }
    workspace.query = "Member 020".into();
    let rows = workspace.rows(&roster, &preferences);
    assert_eq!(rows.len(), 3); // Pool header, owning squad, exact member.
    assert_eq!(
        workspace.selection_state(&roster, &rows),
        SelectionState::Visible
    );
    assert_eq!(workspace.selected, Some(selected));
}

#[test]
fn same_identity_keeps_each_squads_metadata_state_and_attention() {
    let mut roster = roster();
    // The global identity may also have a status; it cannot bleed into room metadata.
    roster.members.get_mut("one").unwrap().state = Some("blocked".into());
    roster.squads[0].member_states.insert(
        "one".into(),
        OpsMembershipState {
            state: Some("blocked".into()),
            waiting: false,
            rank: None,
        },
    );
    let mut other = OpsSquad {
        room_id: "other".into(),
        name: "other".into(),
        ..roster.squads[0].clone()
    };
    other.member_states.insert(
        "one".into(),
        OpsMembershipState {
            state: Some("idle".into()),
            waiting: false,
            rank: None,
        },
    );
    roster.squads.push(other);
    let mut workspace = OpsWorkspace::default();
    let preferences = OpsPreferences::default();
    let collapsed = workspace.rows(&roster, &preferences);
    let other = collapsed
        .iter()
        .find(|row| row.target == OpsTarget::Squad("other".into()))
        .unwrap();
    assert_eq!(other.state.as_deref(), Some("idle"));
    workspace.expanded.extend(["room".into(), "other".into()]);
    let expanded = workspace.rows(&roster, &preferences);
    assert_eq!(
        expanded
            .iter()
            .find(|row| row.target == target(Some("room"), "one"))
            .unwrap()
            .state
            .as_deref(),
        Some("blocked")
    );
    assert_eq!(
        expanded
            .iter()
            .find(|row| row.target == target(Some("other"), "one"))
            .unwrap()
            .state
            .as_deref(),
        Some("idle")
    );
}

#[test]
fn a_collapsed_squad_preserves_its_configured_worst_observed_state() {
    let mut roster = roster();
    roster.squads[0].member_states = [
        (
            "one".into(),
            OpsMembershipState {
                state: Some("hold".into()),
                waiting: false,
                rank: Some((1, false, 0)),
            },
        ),
        (
            "lead".into(),
            OpsMembershipState {
                state: Some("working".into()),
                waiting: false,
                rank: Some((20, false, 1)),
            },
        ),
    ]
    .into();
    let rows = OpsWorkspace::default().rows(&roster, &OpsPreferences::default());
    assert_eq!(rows[1].state.as_deref(), Some("hold"));
    roster.squads[0]
        .member_states
        .get_mut("lead")
        .unwrap()
        .waiting = true;
    assert_eq!(
        OpsWorkspace::default().rows(&roster, &OpsPreferences::default())[1]
            .state
            .as_deref(),
        Some("waiting")
    );
}
