use super::model;
use crate::board::app::workspace::tests::{MEMBER, board};
use crate::board::workspace::{
    OpsMember, OpsObservation, OpsTarget, observe::OpsWorkspaceSnapshot,
};

fn supplied(held: serde_json::Value, stale: bool) -> crate::labels::Supplied {
    crate::labels::Supplied::from_document(
        "digest",
        &serde_json::json!({
            "version": 1, "observedAtMs": 1000,
            "members": [{"identityId":MEMBER, "name":"Agent",
                "mode":"auto", "intervalMs":null, "settingSource":"default", "settingValue":"default",
                "heldCount":held, "statsObservedAtMs":null, "nextSendAtMs":null, "timeToSendMs":null,
            "labels":[{"text":"Auto","colorClass":"text", "action": {
                "kind":"choose", "argv":["digest",MEMBER,"{value}"],
                "options":[{"label":"Default (Auto)","value":"default"},
                    {"label":"Auto","value":"auto"},{"label":"Off","value":"off"}],
                "current":"default"
            }},{"text":"When idle","colorClass":"muted"}]
            }]
        }),
        stale,
    )
}

#[test]
fn null_stats_keep_the_inherited_idle_setting_without_a_zero_count() {
    let supplied = supplied(serde_json::Value::Null, false);
    assert!(model::summary(&supplied).contains("held unavailable"));
    assert!(!model::summary(&supplied).contains("0 held"));
    let details = model::details(&supplied, MEMBER);
    assert!(details.iter().any(|s| s.contains("inherited default")));
    assert!(details.iter().any(|s| s == "Auto: when idle"));
    assert!(!details.iter().any(|s| s.starts_with("Next send")));
}

#[test]
fn known_zero_and_retained_stale_statistics_stay_distinct() {
    let zero = supplied(serde_json::json!(0), false);
    assert!(model::summary(&zero).contains("0 held"));
    let stale = supplied(serde_json::json!(3), true);
    assert!(model::summary(&stale).contains("3 held · stale observation"));
    assert!(
        model::details(&stale, MEMBER)
            .iter()
            .any(|s| s.contains("Stale observation"))
    );
}

#[test]
fn unavailable_statistics_never_become_zero() {
    assert_eq!(
        model::summary(&Default::default()),
        "Digest status unavailable"
    );
    assert!(
        model::details(&Default::default(), "missing")
            .iter()
            .any(|line| line.contains("unavailable"))
    );
}

#[test]
fn duplicate_memberships_have_one_row_and_keep_a_management_context() {
    let mut snapshot = OpsWorkspaceSnapshot::default();
    snapshot.roster.observation = OpsObservation::Complete;
    snapshot.roster.members.insert(
        "id".into(),
        OpsMember {
            id: "id".into(),
            name: "Agent".into(),
            state: None,
            waiting: false,
            presence: None,
            usage: None,
        },
    );
    for room in [None, Some("a"), Some("b")] {
        snapshot.rows.insert(
            OpsTarget::Member {
                room_id: room.map(str::to_owned),
                identity_id: "id".into(),
            },
            serde_json::json!({"id":"id","name":"Agent"}),
        );
    }
    let rows = model::rows(&snapshot, "agent");
    assert_eq!(rows.len(), 1);
    assert!(
        matches!(&rows[0].target, OpsTarget::Member { room_id: Some(room), .. } if room == "a")
    );
    assert!(model::rows(&snapshot, "absent").is_empty());
}

fn paint(app: &mut crate::board::app::App, width: u16, height: u16) -> String {
    use ratatui::{Terminal, backend::TestBackend};
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn press(app: &mut crate::board::app::App, key: char) -> crate::board::app::Effect {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    app.key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE))
}

#[test]
fn shell_dashboard_keeps_the_same_identity_and_uses_the_supplied_picker() {
    use crate::board::app::{Effect, Request};
    let mut app = board();
    app.labels = supplied(serde_json::json!(3), false);
    let selected = app.workspace.selected.clone();
    press(&mut app, '3');
    assert_eq!(app.shell.tab, crate::board::shell::OpsTab::Digest);
    assert_eq!(app.workspace.selected, selected);
    assert_eq!(app.workspace_rows().len(), 1);
    let shown = paint(&mut app, 160, 36);
    assert!(shown.contains("Digest"));
    assert!(shown.contains("Auto ▾"));
    assert!(shown.contains("inherited default"));
    assert!(shown.contains("Auto: when idle"));
    match press(&mut app, 'D') {
        Effect::Act(Request::DigestOpen(ask)) => {
            assert_eq!(ask.squad, "product");
            assert_eq!(ask.member, "worker");
            assert_eq!(ask.choose.current, "default");
            assert_eq!(ask.choose.argv("off").unwrap(), ["digest", MEMBER, "off"]);
        }
        effect => panic!("expected supplier-owned picker admission, got {effect:?}"),
    }
}

#[test]
fn status_loss_and_low_height_do_not_leave_old_chip_pointer_authority() {
    let mut app = board();
    app.labels = supplied(serde_json::json!(3), false);
    press(&mut app, '3');
    paint(&mut app, 160, 36);
    assert!(!app.chip_hits.borrow().is_empty());
    let selected = app.workspace.selected.clone();
    app.labels = Default::default();
    app.invalidate_frame_geometry();
    let shown = paint(&mut app, 60, 8);
    assert_eq!(app.workspace.selected, selected);
    assert!(shown.contains("unavailable"));
    assert!(app.chip_hits.borrow().is_empty());
    paint(&mut app, 80, 18);
    assert!(app.chip_hits.borrow().is_empty());
    assert!(app.digest_choice(app.selected_row().unwrap()).is_none());
}
