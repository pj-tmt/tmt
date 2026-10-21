use super::*;
use crate::{
    board::{
        app::workspace::tests::{MEMBER, ROOM},
        app::{Effect, Request},
        shell::OpsTab,
        workspace::{AgentsFilter, OpsMember, OpsMembershipState, OpsObservation},
    },
    config::{OpsKeyScope, OverviewColumn, OverviewSort},
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
};
use serde_json::json;
const OTHER: &str = "33333333-3333-4333-8333-333333333333";
const BLOCKED: &str = "44444444-4444-4444-8444-444444444444";

#[test]
fn section_counts_use_singular_for_one_and_preserve_filtered_and_partial_counts() {
    for count in [0, 1, 2] {
        let mut app = crate::board::app::workspace::tests::populated_board(count, count);
        app.shell.tab = OpsTab::Agents;
        app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
        for view in [AgentsFilter::All, AgentsFilter::Agents] {
            app.workspace.filter = view;
            app.search.clear();
            app.workspace.selected = None;
            app.workspace_reconcile();
            let buffer = paint(&mut app, 103, 51);
            let shown = text(&buffer);
            if count == 1 {
                assert!(
                    shown.contains(if view == AgentsFilter::All {
                        "1 squad · 1 agent"
                    } else {
                        "1 agent"
                    }),
                    "{shown}"
                );
                assert!(!shown.contains("1 squads") && !shown.contains("1 agents"));
            } else {
                assert!(shown.contains(&format!("{count} agents")), "{shown}");
            }
            app.search = "agent-00000".into();
            app.workspace_reconcile();
            app.overview_rows.borrow_mut().clear();
            let filtered = text(&paint(&mut app, 103, 51));
            assert!(
                filtered.contains(&format!("{} of {count}", usize::from(count > 0))),
                "{filtered}"
            );
            if count == 1 {
                app.view
                    .as_mut()
                    .unwrap()
                    .home
                    .as_mut()
                    .unwrap()
                    .workspace
                    .roster
                    .observation = OpsObservation::Partial;
                app.overview_rows.borrow_mut().clear();
                let partial = text(&paint(&mut app, 103, 51));
                assert!(partial.contains("~1 of ~1"), "{partial}");
                app.view
                    .as_mut()
                    .unwrap()
                    .home
                    .as_mut()
                    .unwrap()
                    .workspace
                    .roster
                    .observation = OpsObservation::Complete;
            }
            app.workspace.query.clear();
        }
    }
}

#[test]
fn agents_use_available_height_before_reporting_overflow() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = crate::board::app::workspace::tests::populated_board(8, 28);
    app.shell.tab = OpsTab::Agents;
    app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
    app.workspace_reconcile();
    for height in [51, 24] {
        let buffer = paint(&mut app, 148, height);
        let lines: Vec<String> = buffer
            .content
            .chunks(148)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect();
        let hits = app.hits.borrow();
        let rows: Vec<_> = hits
            .iter()
            .filter(|hit| matches!(hit.target, crate::board::app::HitTarget::Row(_)))
            .collect();
        if height == 51 {
            assert_eq!(rows.len(), 28);
            assert!(!lines.iter().any(|line| line.contains(" more")));
        } else {
            assert!(!rows.is_empty() && rows.len() < 28);
            let indicator = lines
                .iter()
                .position(|line| line.starts_with(" ↓ "))
                .unwrap();
            // Two footer rows follow the list.
            assert_eq!(indicator, usize::from(height - 3));
            assert!(
                lines[usize::from(rows[0].y)..indicator]
                    .iter()
                    .all(|line| !line.trim().is_empty())
            );
            assert_eq!(usize::from(rows.last().unwrap().y) + 1, indicator);
        }
    }
}

#[test]
fn agents_omit_the_waiting_note_when_none_are_waiting() {
    let mut app = board();
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    for member in snapshot.roster.members.values_mut() {
        member.waiting = false;
    }
    for squad in &mut snapshot.roster.squads {
        for state in squad.member_states.values_mut() {
            state.waiting = false;
        }
    }
    app.overview_rows.borrow_mut().clear();
    let buffer = paint(&mut app, 103, 51);
    let line: String = buffer
        .content
        .chunks(103)
        .find(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .contains("3 agents")
        })
        .unwrap()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(line.trim_end().ends_with("3 agents"));
    assert!(!line.contains("waiting first") && !line.contains('◆'));
}

#[test]
fn filtered_counts_keep_the_admitted_denominator_in_both_views() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = board();
    for view in [AgentsFilter::All, AgentsFilter::Agents] {
        app.workspace.filter = view;
        // Both views count agents against the admitted total.
        let expected = ["1 of 3", "0 of 3"];
        for (query, expected) in ["worker", "no-such-agent"].into_iter().zip(expected) {
            app.search = query.into();
            app.overview_rows.borrow_mut().clear();
            app.workspace_reconcile();
            let buffer = paint(&mut app, 103, 51);
            let lines: Vec<String> = buffer
                .content
                .chunks(103)
                .map(|row| row.iter().map(|cell| cell.symbol()).collect())
                .collect();
            let filter = lines
                .iter()
                .position(|line| line.contains("Agents") && line.contains("Squads"))
                .unwrap();
            assert!(
                lines[filter].contains(expected) && lines[filter].contains('×'),
                "{}",
                lines[filter]
            );
            assert!(!lines[filter + 2].contains(" of "));
            assert!(!lines[filter + 2].contains("waiting first"));
        }
    }
}
/// The squads tree with its first squad selected, as the Squads chip leaves it for a squad.
fn select_squad(app: &mut App) {
    app.agents_filter(AgentsFilter::All);
    app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
    app.workspace_reconcile();
}
fn board() -> App {
    let mut app = crate::board::app::workspace::tests::board();
    app.shell.tab = OpsTab::Agents;
    app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    for member in snapshot.roster.members.values_mut() {
        member.waiting = false;
    }
    for squad in &mut snapshot.roster.squads {
        for member in squad.member_states.values_mut() {
            member.waiting = false;
        }
    }
    let now = crate::status::now_ms();
    for (id, name, state, waiting, age) in [
        (MEMBER, "worker", "working", true, 1_000),
        (OTHER, "waiting-other", "idle", true, 2_000),
        (BLOCKED, "blocked-only", "blocked", false, 0),
    ] {
        snapshot.roster.members.insert(
            id.into(),
            OpsMember {
                id: id.into(),
                name: name.into(),
                state: Some(state.into()),
                waiting,
                presence: Some(true),
                usage: None,
                activity: Default::default(),
            },
        );
        snapshot.roster.squads[0].members.push(id.into());
        snapshot.roster.squads[0].member_states.insert(
            id.into(),
            OpsMembershipState {
                state: Some(state.into()),
                waiting,
                ..Default::default()
            },
        );
        snapshot.rows.insert(OpsTarget::Member{room_id:Some(ROOM.into()),identity_id:id.into()},json!({"id":id,"name":name,"state":state,"presence":"active","pending":null,"fields":{"task":format!("task for {name}")},"waitingOnYou":if waiting{json!([{"requestId":format!("q-{id}"),"preparedAtMs":now-age,"preview":format!("Approve {name}?")}])}else{json!([])}}));
    }
    snapshot.roster.squads[0].members.sort();
    snapshot.roster.squads[0].members.dedup();
    snapshot.shell_counts = snapshot.roster.shell_counts();
    app.overview_rows.borrow_mut().clear();
    app.workspace_reconcile();
    app
}
fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn paint(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}
fn text(buffer: &Buffer) -> String {
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn waiting_is_request_attention_not_a_state_word_or_blocked_status() {
    let mut app = board();
    assert_eq!(
        app.workspace_rows()
            .iter()
            .map(|row| row.title.as_str())
            .collect::<Vec<_>>(),
        ["waiting-other", "worker", "blocked-only"]
    );
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot.roster.members.get_mut(BLOCKED).unwrap().state = Some("waiting".into());
    snapshot.roster.squads[0]
        .member_states
        .get_mut(BLOCKED)
        .unwrap()
        .state = Some("waiting".into());
    app.overview_rows.borrow_mut().clear();
    assert_eq!(
        scene(&app, app.workspace_snapshot().unwrap()).waiting,
        2,
        "an authored word cannot manufacture a pending request"
    );
}
#[test]
fn waiting_order_uses_known_oldest_evidence_then_unknown_without_a_duration_label() {
    let mut app = board();
    assert_eq!(
        app.workspace_rows()[0].title,
        "waiting-other",
        "unpinned waiting rows use their oldest known evidence"
    );
    let raw = app
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .rows
        .get_mut(&OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: OTHER.into(),
        })
        .unwrap();
    raw["waitingOnYou"] = json!([]);
    raw["pending"] = json!("Approve waiting-other?");
    app.overview_rows.borrow_mut().clear();
    assert_eq!(
        app.workspace_rows()[0].title,
        "worker",
        "unknown age follows known evidence"
    );
    let rendered = text(&paint(&mut app, 103, 51));
    assert!(rendered.contains("waiting"));
    assert!(!rendered.contains("waiting 2s"));
}
#[test]
fn cursor_navigation_reuses_the_scene_and_never_selects_read_only_sections() {
    let mut app = board();
    let before = app.workspace_rows();
    let selected = app.workspace.selected.clone();
    assert_eq!(press(&mut app, KeyCode::Up), Effect::None);
    assert_ne!(app.workspace.selected, selected);
    let after = app.workspace_rows();
    assert!(Rc::ptr_eq(&before, &after));
    assert_eq!(app.workspace.selected.as_ref(), Some(&before[0].target));
    assert!(matches!(
        app.workspace.selected,
        Some(OpsTarget::Member { .. })
    ));
}
#[test]
fn search_keeps_a_hidden_uuid_and_authored_filter_bindings_win() {
    let mut app = board();
    let selected = app.workspace.selected.clone();
    app.search = "not present".into();
    app.workspace_reconcile();
    assert!(app.workspace_rows().is_empty());
    assert_eq!(app.workspace.selected, selected);
    assert!(!app.workspace_selection_visible());
    press(&mut app, KeyCode::Enter);
    assert!(app.input.is_none());
    assert_eq!(app.workspace.selected, selected);
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .keys
        .get_mut(&OpsKeyScope::Member)
        .unwrap()
        .insert("v".into(), crate::action::Action::parse("search").unwrap());
    press(&mut app, KeyCode::Char('v'));
    assert!(app.searching);
    assert!(app.workspace.filter == AgentsFilter::Agents);
}
#[test]
fn authored_sort_changes_only_the_layer_order_and_retains_the_selected_uuid() {
    let mut app = board();
    let selected = app.workspace.selected.clone();
    let preferences = app.workspace_snapshot().unwrap().preferences.clone();
    let keys = app
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .keys
        .get_mut(&OpsKeyScope::Member)
        .unwrap();
    keys.insert("g".into(), crate::action::Action::parse("sort").unwrap());
    keys.insert(
        "G".into(),
        crate::action::Action::parse("reverse-sort").unwrap(),
    );
    assert_eq!(press(&mut app, KeyCode::Char('g')), Effect::None);
    assert_eq!(
        app.workspace_order().overview,
        Some(OverviewSort::initial(OverviewColumn::Agent))
    );
    assert_eq!(press(&mut app, KeyCode::Char('G')), Effect::None);
    assert_eq!(
        app.workspace_order().overview,
        Some(OverviewSort::initial(OverviewColumn::Agent).reverse())
    );
    assert_eq!(app.workspace.selected, selected);
    assert_eq!(app.workspace_snapshot().unwrap().preferences, preferences);
}
#[test]
fn real_frames_publish_only_clipped_agent_hits_and_neutral_state_words() {
    for width in [60, 103, 148, 160] {
        for height in [8, 18, 36, 51] {
            let mut app = board();
            let _no_commands = crate::runner::forbid_commands();
            let buffer = paint(&mut app, width, height);
            let rows = app.workspace_rows();
            let hits: Vec<_> = app
                .hits
                .borrow()
                .iter()
                .filter_map(|hit| hit.row().map(|index| (index, *hit)))
                .collect();
            assert!(!hits.is_empty(), "{width}x{height} retains an admitted row");
            let mut ys = std::collections::BTreeSet::new();
            for (index, hit) in hits {
                assert!(index < rows.len());
                assert!(hit.x + hit.width <= width && hit.y < height);
                assert!(ys.insert(hit.y));
                let label = app.activity_label(&rows[index]).expect("a member row");
                assert!(!label.state.is_empty());
                if hit.width >= 60 {
                    // The shared STATE words stay neutral text; only the glyph
                    // carries attention colour. Unknown activity has no word, so
                    // its task follows the glyph: red only when blocked.
                    let role = match (label.state, label.task.as_deref()) {
                        ("unknown", Some("blocked")) => tmt_cli_style::Role::Blocked,
                        ("unknown", _) => tmt_cli_style::Role::Muted,
                        _ => tmt_cli_style::Role::Text,
                    };
                    assert_eq!(
                        buffer[(hit.x + 39, hit.y)].fg,
                        app.look().role(role).fg.unwrap_or_default(),
                        "{label:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn refreshed_and_hidden_frames_revoke_the_previous_row_hits() {
    let mut app = board();
    paint(&mut app, 103, 51);
    assert!(
        !app.hits
            .borrow()
            .iter()
            .filter_map(|hit| hit.row())
            .collect::<Vec<_>>()
            .is_empty()
    );
    app.search = "missing".into();
    app.workspace_reconcile();
    paint(&mut app, 60, 18);
    assert!(app.hits.borrow().iter().all(|hit| hit.row().is_none()));
    assert!(text(&paint(&mut app, 103, 51)).contains("No matching agents"));
}
#[test]
fn missing_sources_stay_unknown_instead_of_claiming_zero_waiting() {
    let mut app = board();
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    for member in snapshot.roster.members.values_mut() {
        member.waiting = false;
    }
    for squad in &mut snapshot.roster.squads {
        for state in squad.member_states.values_mut() {
            state.waiting = false;
        }
        squad.observation = OpsObservation::Unavailable;
    }
    snapshot.roster.observation = OpsObservation::Unavailable;
    snapshot.roster.membership_complete = false;
    app.overview_rows.borrow_mut().clear();
    let shown = text(&paint(&mut app, 103, 51));
    assert!(shown.contains("~3 agents"));
    assert!(!shown.contains("waiting first"));
    assert!(shown.contains("AGENT") && !shown.contains("Needs you"));
    assert!(!shown.contains("◆ 0 waiting"));
}
#[test]
fn final_primary_sections_replace_the_legacy_summaries_and_recent_table() {
    let mut app = board();
    app.agents_filter(crate::board::workspace::AgentsFilter::All);
    let squads = text(&paint(&mut app, 103, 51));
    for required in ["Squads", "SQUAD / AGENT", "TOK 1h", "MODEL", "/  Search"] {
        assert!(squads.contains(required), "{required}");
    }
    app.agents_filter(crate::board::workspace::AgentsFilter::Agents);
    let agents = text(&paint(&mut app, 103, 51));
    for required in ["AGENT", "SQUAD", "STATE", "TOK 1h", "MODEL"] {
        assert!(agents.contains(required), "{required}");
    }
    for retired in [
        "Recent exchanges",
        "Digest status unavailable",
        "state unknown",
        "Needs you",
        "f needs you",
        "open chat",
    ] {
        assert!(
            !squads.contains(retired) && !agents.contains(retired),
            "retired {retired}"
        );
    }
    for shown in [&squads, &agents] {
        assert!(!shown.split_whitespace().any(|word| word == "MODE"));
    }
}

#[test]
fn pin_requests_use_agent_identity_and_success_only_sanitized_text() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = board();
    let row = app
        .workspace_rows()
        .iter()
        .find(|row| row.title != "A lead")
        .unwrap()
        .target
        .clone();
    app.workspace.selected = Some(row.clone());
    let OpsTarget::Member { identity_id, .. } = row else {
        unreachable!()
    };
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot.roster.members.get_mut(&identity_id).unwrap().name = "Safe\nname\u{1b}[31m".into();
    app.overview_rows.borrow_mut().clear();
    let selected = app.workspace.selected.clone();
    for (word, should_pin) in [("Pinned", true), ("Unpinned", false)] {
        let effect = app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        let Effect::Act(Request::WorkspacePreferences { next, saved, .. }) = effect else {
            panic!("pin requires the existing save lane")
        };
        assert_eq!(saved.as_deref(), Some(format!("{word} Safename").as_str()));
        assert_eq!(next.agent_pins.contains(&identity_id), should_pin);
        assert!(next.pinned.is_empty());
        assert_eq!(app.workspace.selected, selected);
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .preferences = next;
        app.overview_rows.borrow_mut().clear();
    }
    app.workspace.query = "no matching agent".into();
    assert!(!matches!(
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE)),
        Effect::Act(_)
    ));
    assert_eq!(app.workspace.selected, selected);
}

#[test]
fn an_ineligible_preferred_membership_cannot_mask_a_waiting_occurrence() {
    let mut app = board();
    let preferred = OpsTarget::Member {
        room_id: Some(ROOM.into()),
        identity_id: MEMBER.into(),
    };
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let mut other = snapshot.roster.squads[0].clone();
    other.room_id = "99999999-9999-4999-8999-999999999999".into();
    other.name = "other-room".into();
    other.members = vec![MEMBER.into()];
    other.lead = Some(MEMBER.into());
    other.member_states.retain(|id, _| id == MEMBER);
    other.member_states.get_mut(MEMBER).unwrap().waiting = true;
    let eligible = OpsTarget::Member {
        room_id: Some(other.room_id.clone()),
        identity_id: MEMBER.into(),
    };
    let row = snapshot.rows[&preferred].clone();
    snapshot.rows.insert(eligible.clone(), row);
    snapshot.roster.squads[0]
        .member_states
        .get_mut(MEMBER)
        .unwrap()
        .waiting = false;
    snapshot.roster.squads.push(other);
    let rows = crate::board::workspace::identity_rows_matching(
        snapshot,
        "worker",
        Some(&preferred),
        &Default::default(),
        |row| waits(snapshot, row),
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].target, eligible);
    let unfiltered = crate::board::workspace::identity_rows(
        snapshot,
        "worker",
        Some(&preferred),
        &Default::default(),
    );
    assert_eq!(
        unfiltered[0].target, preferred,
        "positive control: preference still selects an eligible occurrence"
    );
}

#[test]
fn global_waiting_does_not_attribute_an_unobserved_room_membership() {
    let mut app = board();
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    assert!(snapshot.roster.members[MEMBER].waiting);
    snapshot.roster.squads[0].member_states.remove(MEMBER);
    app.overview_rows.borrow_mut().clear();
    let snapshot = app.workspace_snapshot().unwrap();
    let scene = scene(&app, snapshot);
    let row = scene.rows.iter().find(|row| matches!(&row.target, OpsTarget::Member { identity_id, .. } if identity_id == MEMBER)).expect("all-agent view retains the observed identity");
    assert!(
        !waits(snapshot, row),
        "global waiting must not be attributed to this unobserved membership"
    );
    assert_eq!(scene.waiting, 1);
    assert!(
        snapshot.roster.shell_counts().waiting > 0,
        "global evidence stays in its own scope"
    );
}

#[test]
fn view_toggle_and_painted_chips_share_selection_while_authored_v_wins() {
    let mut app = board();
    select_squad(&mut app);
    assert!(matches!(app.workspace.selected, Some(OpsTarget::Squad(_))));
    press(&mut app, KeyCode::Char('f'));
    assert_eq!(app.workspace.filter, AgentsFilter::Agents);
    assert!(matches!(
        app.workspace.selected,
        Some(OpsTarget::Member { .. })
    ));
    paint(&mut app, 103, 51);
    let (area, _) = app
        .workspace_strip_hits
        .borrow()
        .iter()
        .find(|(_, choice)| {
            matches!(
                choice,
                crate::board::app::Choice::AgentsFilter(crate::board::workspace::AgentsFilter::All)
            )
        })
        .unwrap()
        .clone();
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    assert_eq!(app.workspace.filter, AgentsFilter::All);
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .keys
        .get_mut(&OpsKeyScope::Member)
        .unwrap()
        .insert("v".into(), crate::action::Action::parse("search").unwrap());
    press(&mut app, KeyCode::Char('v'));
    assert!(app.searching);
    assert_eq!(
        app.workspace.filter,
        AgentsFilter::All,
        "authored v must not toggle the view"
    );
}

#[test]
fn squad_message_target_is_only_its_current_observed_lead() {
    let mut app = board();
    select_squad(&mut app);
    assert!(
        app.workspace_message_target().is_none(),
        "no lead means no recipient"
    );
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .squads[0]
        .lead = Some(MEMBER.into());
    app.overview_rows.borrow_mut().clear();
    let target = app.workspace_message_target().expect("observed lead");
    let OpsTarget::Member {
        room_id: Some(room),
        identity_id: lead,
    } = &target
    else {
        panic!("canonical room member")
    };
    let snapshot = app.workspace_snapshot().unwrap();
    assert_eq!(
        snapshot
            .roster
            .squads
            .iter()
            .find(|s| &s.room_id == room)
            .unwrap()
            .lead
            .as_ref(),
        Some(lead)
    );
    assert_eq!(
        app.conversation_target(),
        Some(crate::board::app::RowTarget::Workspace(target.clone()))
    );
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .rows
        .remove(&target);
    assert!(
        app.workspace_message_target().is_none(),
        "missing lead evidence cannot retarget to a neighbor"
    );
    assert!(app.conversation_target().is_none());
}

#[test]
fn hidden_empty_lists_are_filtered_and_missing_sources_never_claim_a_fresh_empty() {
    let mut app = board();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .hidden
        .insert(ROOM.into());
    app.overview_rows.borrow_mut().clear();
    app.agents_filter(crate::board::workspace::AgentsFilter::All);
    let squads = text(&paint(&mut app, 103, 51));
    // The hidden squad leaves the tree; the common pool stays.
    assert!(
        !squads.contains("product") && squads.contains("Common pool"),
        "{squads}"
    );
    assert!(!squads.contains("No squads yet."));
    app.agents_filter(crate::board::workspace::AgentsFilter::Agents);
    assert!(text(&paint(&mut app, 103, 51)).contains("No matching agents"));
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .observation = OpsObservation::Unavailable;
    app.overview_rows.borrow_mut().clear();
    assert!(text(&paint(&mut app, 103, 51)).contains("Agents unavailable"));
}

#[test]
fn filtered_section_counts_follow_the_current_squads_or_agents_mode() {
    let mut app = crate::board::app::workspace::tests::populated_board(8, 28);
    app.shell.tab = OpsTab::Agents;
    app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
    app.search = "squad-00".into();
    app.workspace_reconcile();
    app.agents_filter(crate::board::workspace::AgentsFilter::All);
    let squads = text(&paint(&mut app, 103, 51));
    assert!(squads.contains("4 of 28"), "{squads}");
    app.search = "agent-00000".into();
    app.workspace_reconcile();
    app.agents_filter(crate::board::workspace::AgentsFilter::Agents);
    let agents = text(&paint(&mut app, 103, 51));
    assert!(agents.contains("1 of 28"), "{agents}");
}

#[test]
fn explicit_agent_pins_sort_before_attention_without_duplicating_memberships() {
    let mut app = board();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .agent_pins
        .insert(BLOCKED.into());
    let snapshot = app.workspace_snapshot().unwrap();
    let scene = scene(&app, snapshot);
    assert!(
        matches!(&scene.rows[0].target, crate::board::workspace::OpsTarget::Member { identity_id, .. } if identity_id == BLOCKED)
    );
    let ids = scene
        .rows
        .iter()
        .filter_map(|row| match &row.target {
            crate::board::workspace::OpsTarget::Member { identity_id, .. } => Some(identity_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ids.len(),
        ids.iter().collect::<std::collections::BTreeSet<_>>().len()
    );
}
