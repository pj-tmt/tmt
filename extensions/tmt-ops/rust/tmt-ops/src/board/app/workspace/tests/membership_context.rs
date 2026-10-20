//! Flat identities resolve their observed memberships without changing selection.
use super::*;

const SECOND_ROOM: &str = "44444444-4444-4444-8444-444444444444";

fn flat(memberships: usize, tab: OpsTab) -> App {
    let mut app = board();
    observe_waiting_member(&mut app);
    let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let row = workspace.rows[&OpsTarget::Member {
        room_id: Some(ROOM.into()),
        identity_id: MEMBER.into(),
    }]
        .clone();
    workspace.rows.insert(
        OpsTarget::Member {
            room_id: None,
            identity_id: MEMBER.into(),
        },
        row.clone(),
    );
    if memberships == 0 {
        workspace.roster.squads.clear();
    } else if memberships == 2 {
        let mut second = workspace.roster.squads[0].clone();
        second.room_id = SECOND_ROOM.into();
        second.name = "research".into();
        workspace.roster.squads.push(second);
        workspace.rows.insert(
            OpsTarget::Squad(SECOND_ROOM.into()),
            json!({"name":"research","roomId":SECOND_ROOM,"revision":7,"memberIds":[MEMBER]}),
        );
        workspace.rows.insert(
            OpsTarget::Member {
                room_id: Some(SECOND_ROOM.into()),
                identity_id: MEMBER.into(),
            },
            row,
        );
    }
    app.shell.tab = tab;
    app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
    app.workspace.pool_expanded = memberships == 0;
    app.overview_rows.borrow_mut().clear();
    app.workspace.selected = app
        .workspace_rows()
        .iter()
        .find(|row| matches!(&row.target, OpsTarget::Member { identity_id, .. } if identity_id == MEMBER))
        .map(|row| row.target.clone());
    app.workspace_reconcile();
    paint(&app, 160, 36);
    app
}

fn assert_remove(app: &App, room_id: &str, revision: u64) {
    let draft = app.workspace_form.as_ref().expect("membership review");
    assert_eq!(draft.stage, Stage::Review);
    assert!(matches!(
        draft.operation.as_ref(),
        Some(crate::board::workspace::manage::Change::Membership(
            crate::membership::workspace::Operation::Remove { room, member }
        )) if room.id == room_id && room.revision == revision && member == MEMBER
    ));
}

#[test]
fn flat_lists_remove_the_only_membership_and_keep_the_selected_uuid() {
    for tab in [OpsTab::Agents, OpsTab::Digest] {
        let mut app = flat(1, tab);
        let selected = app.workspace.selected.clone();
        assert_eq!(key(&mut app, KeyCode::Char('x')), Effect::None);
        assert_remove(&app, ROOM, 2);
        assert_eq!(app.workspace.selected, selected);
        let text = paint(&app, 100, 24).join("\n");
        assert!(text.contains("From squad: product"), "{text}");
        assert_eq!(key(&mut app, KeyCode::Esc), Effect::None);
        assert!(app.workspace_form.is_none());
        assert_eq!(app.workspace.selected, selected);
    }
}

#[test]
fn multiple_memberships_require_a_room_choice_before_any_review() {
    for tab in [OpsTab::Agents, OpsTab::Digest] {
        let mut app = flat(2, tab);
        let selected = app.workspace.selected.clone();
        assert_eq!(key(&mut app, KeyCode::Char('x')), Effect::None);
        assert!(app.workspace_form.is_none());
        let menu = app.menu.as_ref().expect("explicit squad picker");
        assert_eq!(menu.title, "Remove worker from:");
        assert_eq!(menu.entries.len(), 2);
        assert_eq!(menu.entries[0].label, "product");
        assert_eq!(menu.entries[1].label, "research");
        assert_eq!(app.workspace.selected, selected);
        let text = paint(&app, 80, 24).join("\n");
        assert!(text.contains("› product"), "{text}");
        assert!(text.contains("Enter remove · Esc cancel"), "{text}");
        assert!(!text.contains("Enter runs"), "{text}");
        assert_eq!(key(&mut app, KeyCode::Down), Effect::None);
        assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
        assert_remove(&app, SECOND_ROOM, 7);
        assert_eq!(app.workspace.selected, selected);
    }
}

#[test]
fn common_pool_member_has_no_membership_instead_of_a_refresh_problem() {
    for tab in [OpsTab::Agents] {
        let mut app = flat(0, tab);
        app.shell.tab = tab;
        app.workspace.pool_expanded = true;
        app.workspace_reconcile();
        paint(&app, 100, 24);
        assert_eq!(key(&mut app, KeyCode::Char('x')), Effect::None);
        let notice = app.notice.as_deref().expect("known membership absence");
        assert!(notice.contains("worker is not in a squad"), "{notice}");
        assert!(!notice.contains("refresh"), "{notice}");
        assert!(app.workspace_form.is_none() && app.menu.is_none());
    }
}

#[test]
fn a_flat_partial_roster_needs_refresh_even_with_one_observed_membership() {
    for tab in [OpsTab::Agents, OpsTab::Digest] {
        for observation in [OpsObservation::Partial, OpsObservation::Unavailable] {
            let mut app = flat(1, tab);
            let roster = &mut app
                .view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .roster;
            roster.observation = observation;
            roster.membership_complete = false;
            app.workspace_reconcile();
            paint(&app, 160, 36);
            assert_eq!(key(&mut app, KeyCode::Char('x')), Effect::None);
            assert!(app.workspace_form.is_none() && app.menu.is_none());
            assert!(app.notice.as_deref().unwrap().contains("refresh"));

            let mut fresh = flat(1, tab);
            let mut publication = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
            publication.tabs.push(crate::board::ALL.into());
            publication.view = Ok(fresh.view.take().unwrap());
            app.apply(publication);
            paint(&app, 160, 36);
            assert_eq!(key(&mut app, KeyCode::Char('x')), Effect::None);
            assert_remove(&app, ROOM, 2);
            assert!(app.notice.is_none());
        }
    }
}

#[test]
fn explicit_agents_membership_needs_only_its_complete_room() {
    let mut app = board();
    let roster = &mut app
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster;
    roster.membership_complete = false;
    roster.observation = OpsObservation::Partial;
    paint(&app, 160, 36);
    assert_eq!(key(&mut app, KeyCode::Char('x')), Effect::None);
    assert_remove(&app, ROOM, 2);
}

#[test]
fn agents_choose_the_exact_second_membership_without_flat_resolution() {
    let mut app = flat(2, OpsTab::Agents);
    app.workspace.expanded.insert(SECOND_ROOM.into());
    app.workspace.selected = Some(OpsTarget::Member {
        room_id: Some(SECOND_ROOM.into()),
        identity_id: MEMBER.into(),
    });
    app.workspace_reconcile();
    paint(&app, 100, 24);
    assert_eq!(key(&mut app, KeyCode::Char('x')), Effect::None);
    assert_remove(&app, SECOND_ROOM, 7);
    assert!(app.menu.is_none());
}

#[test]
fn picker_cancel_and_destination_keys_never_apply_a_membership() {
    for exit in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('2')] {
        let mut app = flat(2, OpsTab::Agents);
        let selected = app.workspace.selected.clone();
        key(&mut app, KeyCode::Char('x'));
        paint(&app, 60, 18);
        assert_eq!(key(&mut app, exit), Effect::None);
        assert!(app.menu.is_none() && app.workspace_form.is_none());
        assert_eq!(app.workspace.selected, selected);
        assert_eq!(
            app.shell.tab,
            if exit == KeyCode::Char('2') {
                OpsTab::Digest
            } else {
                OpsTab::Agents
            }
        );
    }
}

#[test]
fn choosing_revalidates_membership_selection_and_complete_evidence() {
    for change in [
        "membership",
        "member",
        "room",
        "selected",
        "hidden",
        "partial",
        "missing-details",
    ] {
        let mut app = flat(2, OpsTab::Agents);
        key(&mut app, KeyCode::Char('x'));
        let choice = app.menu.as_ref().unwrap().entries[1].choice.clone();
        app.menu = None;
        let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        match change {
            "membership" => workspace.roster.squads[1].members.clear(),
            "member" => {
                workspace.roster.members.remove(MEMBER);
            }
            "room" => {
                workspace.roster.squads.pop();
            }
            "selected" => app.workspace.selected = Some(OpsTarget::Squad(ROOM.into())),
            "hidden" => app.workspace.query = "no-visible-agent".into(),
            "partial" => workspace.roster.membership_complete = false,
            "missing-details" => {
                workspace.rows.remove(&OpsTarget::Squad(SECOND_ROOM.into()));
            }
            _ => unreachable!(),
        }
        app.overview_rows.borrow_mut().clear();
        assert_eq!(app.choose(choice), Effect::None, "{change}");
        assert!(
            app.workspace_form.is_none() && app.menu.is_none(),
            "{change}"
        );
        assert!(app.notice.is_some(), "{change}");
    }
}

#[test]
fn chosen_room_freezes_current_revision_and_only_its_member_timer_impact() {
    let mut app = flat(2, OpsTab::Agents);
    let mut jobs = crate::board::cronboard::test_views(2, "research", SECOND_ROOM);
    for job in &mut jobs {
        job.job.owner_id = Some(MEMBER.into());
    }
    let mut other = crate::board::cronboard::test_views(1, "product", ROOM);
    other[0].job.owner_id = Some(MEMBER.into());
    jobs.extend(other);
    app.cron.replace(Ok(crate::board::cronboard::test_cron(
        jobs,
        tmt_ops::cron::ClockStatus::NoClock,
    )));
    key(&mut app, KeyCode::Char('x'));
    let choice = app.menu.as_ref().unwrap().entries[1].choice.clone();
    assert!(
        matches!(&choice, Choice::WorkspaceRoom {kind:Kind::Remove,identity_id,room_id} if identity_id==MEMBER && room_id==SECOND_ROOM)
    );
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .rows
        .get_mut(&OpsTarget::Squad(SECOND_ROOM.into()))
        .unwrap()["revision"] = json!(8);
    app.menu = None;
    assert_eq!(app.choose(choice), Effect::None);
    assert_remove(&app, SECOND_ROOM, 8);
    assert_eq!(app.workspace_form.as_ref().unwrap().timer_impact, Some(2));
    assert_eq!(app.workspace_confirm(), Effect::None);
    let text = paint(&app, 80, 24).join("\n");
    assert!(text.contains("Source timers to pause: 2"), "{text}");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Tab);
    let effect = key(&mut app, KeyCode::Enter);
    assert!(
        matches!(effect, Effect::Act(Request::WorkspaceOperation(crate::board::workspace::manage::Change::Membership(crate::membership::workspace::Operation::Remove {room,member}))) if room.id==SECOND_ROOM && room.revision==8 && member==MEMBER)
    );
    assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Pending);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_eq!(app.workspace_confirm(), Effect::None);
    app.workspace_close();
    assert_eq!(app.workspace_begin(Kind::Add), Effect::None);
    assert!(app.menu.is_none());
    assert_eq!(app.workspace_form.as_ref().unwrap().stage, Stage::Pending);
    assert_eq!(app.workspace_begin(Kind::Remove), Effect::None);
    assert!(app.workspace_form.as_ref().unwrap().shown);
    assert_eq!(app.workspace_confirm(), Effect::None);
}

#[test]
fn all_squad_required_actions_share_flat_context_and_known_absence_reasons() {
    for kind in [Kind::Add, Kind::Lead, Kind::RenameSquad] {
        for tab in [OpsTab::Agents, OpsTab::Digest] {
            let mut app = flat(0, tab);
            assert_eq!(app.workspace_begin(kind), Effect::None);
            assert!(app.workspace_form.is_none() && app.menu.is_none());
            let notice = app.notice.as_deref().unwrap();
            assert!(
                notice.contains("select a squad") && !notice.contains("refresh"),
                "{notice}"
            );
        }
        let mut one = flat(1, OpsTab::Agents);
        assert_eq!(one.workspace_begin(kind), Effect::None);
        assert_eq!(
            one.workspace_form
                .as_ref()
                .unwrap()
                .room
                .as_ref()
                .unwrap()
                .id,
            ROOM
        );
        let mut multiple = flat(2, OpsTab::Digest);
        assert_eq!(multiple.workspace_begin(kind), Effect::None);
        assert!(multiple.workspace_form.is_none());
        let choice = multiple.menu.as_ref().unwrap().entries[1].choice.clone();
        multiple.menu = None;
        assert_eq!(multiple.choose(choice), Effect::None);
        assert_eq!(
            multiple
                .workspace_form
                .as_ref()
                .unwrap()
                .room
                .as_ref()
                .unwrap()
                .id,
            SECOND_ROOM
        );
    }
}

#[test]
fn incomplete_room_names_its_actual_observation_or_held_read_failure() {
    for observation in [
        OpsObservation::Partial,
        OpsObservation::Stale,
        OpsObservation::Unavailable,
    ] {
        let mut app = board();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .roster
            .squads[0]
            .observation = observation;
        assert_eq!(app.workspace_begin(Kind::Remove), Effect::None);
        let notice = app.notice.as_deref().unwrap();
        assert!(
            notice.contains("product") && notice.contains("refresh"),
            "{notice}"
        );
        assert!(
            notice.contains(match observation {
                OpsObservation::Partial => "partial",
                OpsObservation::Stale => "stale",
                OpsObservation::Unavailable => "unavailable",
                _ => unreachable!(),
            }),
            "{notice}"
        );
        assert!(app.workspace_form.is_none() && app.menu.is_none());
    }
    let mut app = flat(1, OpsTab::Agents);
    let home = app.view.as_mut().unwrap().home.as_mut().unwrap();
    home.workspace.roster.membership_complete = false;
    home.failures.push(json!({"squad":"product", "error":{"code":"BUSY", "message":"room lock is busy\ntry later\u{001b}[31m"}}));
    assert_eq!(app.workspace_begin(Kind::Remove), Effect::None);
    let notice = app.notice.as_deref().unwrap();
    assert!(
        notice.contains("Couldn't read product") && notice.contains("room lock is busy"),
        "{notice}"
    );
    assert!(
        !notice.contains('\n') && !notice.contains('\u{001b}'),
        "{notice}"
    );
}

#[test]
fn picker_pointer_uses_only_current_painted_entry_geometry() {
    let mut app = flat(2, OpsTab::Agents);
    key(&mut app, KeyCode::Char('x'));
    let text = paint(&app, 80, 24);
    let (y, line) = text
        .iter()
        .enumerate()
        .find(|(_, line)| line.contains("research"))
        .unwrap();
    let x = line.find("research").unwrap();
    let pointer = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x as u16,
        row: y as u16,
        modifiers: KeyModifiers::NONE,
    };
    app.invalidate_frame_geometry();
    assert_eq!(
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                ..pointer
            },
            Instant::now()
        ),
        Effect::None
    );
    assert_eq!(app.menu.as_ref().unwrap().selected, 0);
    assert_eq!(app.mouse(pointer, Instant::now()), Effect::None);
    assert!(app.menu.is_some() && app.workspace_form.is_none());
    paint(&app, 80, 24);
    assert_eq!(app.mouse(pointer, Instant::now()), Effect::None);
    assert_remove(&app, SECOND_ROOM, 7);
}

#[test]
fn picker_keys_work_after_geometry_invalidation_without_a_repaint() {
    let mut app = flat(2, OpsTab::Agents);
    key(&mut app, KeyCode::Char('x'));
    paint(&app, 80, 24);
    app.invalidate_frame_geometry();
    assert_eq!(key(&mut app, KeyCode::Down), Effect::None);
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert_remove(&app, SECOND_ROOM, 7);
}

#[test]
fn unknown_membership_absence_is_not_reported_as_a_known_empty_pool() {
    for tab in [OpsTab::Agents, OpsTab::Digest] {
        let mut app = flat(0, tab);
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .roster
            .membership_complete = false;
        app.overview_rows.borrow_mut().clear();
        assert_eq!(
            app.workspace_selection_state(),
            crate::board::workspace::SelectionState::Unknown
        );
        assert_eq!(app.workspace_begin(Kind::Remove), Effect::None);
        let notice = app.notice.as_deref().unwrap();
        assert_eq!(notice, "Select a visible target explicitly.");
        assert!(!notice.contains("is not in a squad"), "{notice}");
        assert!(app.workspace_form.is_none() && app.menu.is_none());
    }
}

#[test]
fn picker_order_is_independent_of_roster_acquisition_order() {
    let mut app = flat(2, OpsTab::Agents);
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .squads
        .reverse();
    app.overview_rows.borrow_mut().clear();
    app.workspace.selected = app
        .workspace_rows()
        .iter()
        .find(|row| matches!(&row.target,OpsTarget::Member {identity_id,..} if identity_id==MEMBER))
        .map(|row| row.target.clone());
    let selected = app.workspace.selected.clone();
    assert_eq!(app.workspace_begin(Kind::Remove), Effect::None);
    let entries = &app.menu.as_ref().unwrap().entries;
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.label.as_str())
            .collect::<Vec<_>>(),
        ["product", "research"]
    );
    assert!(entries.iter().all(|entry| entry.key.is_empty()));
    assert_eq!(app.workspace.selected, selected);
}
