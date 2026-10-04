use super::*;
use crate::action::Action;
use crate::board::{
    app::{App, Effect, Request, Snapshot},
    home::paint,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyCode::*, KeyEvent, KeyModifiers},
    layout::Rect,
    widgets::Paragraph,
};

pub(super) fn press(app: &mut App, key: KeyCode) -> Effect {
    app.key(KeyEvent::new(key, KeyModifiers::NONE))
}
pub(super) fn board(documents: &[(&str, Value)]) -> App {
    let fixture = Fixture::new("");
    let acquired = fixture.acquired(documents);
    let mut order = documents
        .iter()
        .map(|(name, _)| (*name).into())
        .collect::<Vec<String>>();
    let mut snapshot = crate::board::app::tests::snapshot(ALL, json!([]));
    let view = snapshot.view.as_mut().unwrap();
    view.document = tab_view::all_document(
        &order,
        &acquired.documents,
        &tab_view::tab_attention(&acquired.documents),
    );
    view.home = Some(model(&order, &acquired, 100));
    view.bindings = crate::action::all_preset();
    view.me = Some("ben".into());
    order.push(ALL.into());
    snapshot.tabs = order;
    let mut app = App::new(Some(ALL.into()));
    app.apply(snapshot);
    app
}
pub(super) fn keys(app: &mut App, codes: &[KeyCode]) -> Effect {
    codes.iter().fold(Effect::None, |_, code| press(app, *code))
}

pub(super) fn waiting() -> Value {
    let mut member = row("W", "worker", "blocked");
    member["waitingOnYou"] = json!([{"requestId":"q1","preparedAtMs":20,"preview":"private question one"},{"requestId":"q2","preparedAtMs":30,"preview":"private question two"}]);
    document("a", row("L", "lead-a", "working"), vec![member])
}

#[test]
fn one_cursor_moves_across_rows_and_sections_and_enter_goes_in() {
    let mut app = board(&[("a", waiting())]);
    app.view
        .as_mut()
        .unwrap()
        .bindings
        .insert("L".into(), Action::parse("jump lead").unwrap());
    assert_eq!(
        press(&mut app, Char('L')),
        Effect::Act(Request::Jump("lead-a".into()))
    );
    assert_eq!(
        press(&mut app, Enter),
        Effect::Act(Request::Jump("worker".into()))
    );
    assert_eq!(press(&mut app, Char('j')), Effect::None);
    assert_eq!(app.home_target.as_ref().unwrap().section, "blocked");
    press(&mut app, Down);
    assert_eq!(app.home_target.as_ref().unwrap().section, "squads");
    assert_eq!(press(&mut app, Enter), Effect::Load("a".into()));
    let mut app = board(&[("a", waiting())]);
    press(&mut app, Tab);
    assert_eq!(app.home_target.as_ref().unwrap().section, "squads");
    press(&mut app, Tab);
    assert_eq!(app.home_target.as_ref().unwrap().section, "needs-you");
    press(&mut app, BackTab);
    assert_eq!(app.home_target.as_ref().unwrap().section, "squads");
    press(&mut app, BackTab);
    assert_eq!(app.home_target.as_ref().unwrap().section, "needs-you");
    for key in [Char('r'), Char('R'), Char('1')] {
        assert_eq!(press(&mut app, key), Effect::None);
        assert!(app.input.is_none());
    }
    let mut quiet = waiting();
    quiet["sections"][0]["rows"][0]["waitingOnYou"] = json!([]);
    assert_eq!(
        board(&[("a", quiet)]).home_target.unwrap().section,
        "squads"
    );
}

#[test]
fn refresh_and_search_reconcile_the_stable_target() {
    let mut doc = waiting();
    let mut second = row("Z", "zebra", "working");
    second["pending"] = json!("note");
    doc["sections"][0]["rows"]
        .as_array_mut()
        .unwrap()
        .push(second);
    let mut app = board(&[("a", doc.clone())]);
    press(&mut app, Down);
    let target = app.home_target.clone();
    let mut first = row("A", "aardvark", "working");
    first["pending"] = json!("note");
    doc["sections"][0]["rows"]
        .as_array_mut()
        .unwrap()
        .insert(0, first);
    let refreshed = board(&[("a", doc)]);
    app.apply(Snapshot {
        squad_keys: vec!["a".into()],
        tabs: app.tabs.clone(),
        hidden: vec![],
        pinned: 0,
        attention: Default::default(),
        squad: Some(ALL.into()),
        view: Ok(refreshed.view.unwrap()),
    });
    assert_eq!(app.home_target, target);
    assert_eq!(app.selected_row().unwrap()["name"], "zebra");
    keys(&mut app, &[Char('/'), Char('z'), Enter]);
    assert_eq!(app.home_target, target);
    press(&mut app, Esc);
    assert_eq!(app.home_target, target);
    app.view = board(&[("a", document("a", Value::Null, vec![]))]).view;
    press(&mut app, Down);
    assert_eq!(app.selected, 0);
    assert_eq!(app.home_target.unwrap().section, "squads");
}

#[test]
fn answers_use_real_requests_and_submission_rechecks_opening_authority() {
    let mut app = board(&[("a", waiting())]);
    press(&mut app, Char('a'));
    assert_eq!(
        app.menu.as_ref().unwrap().entries[1].label,
        "private question two"
    );
    keys(&mut app, &[Down, Enter, Char('y')]);
    assert_eq!(
        press(&mut app, Enter),
        Effect::Act(Request::Reply {
            me: "ben".into(),
            request: "q2".into(),
            from: "worker".into(),
            text: "y".into()
        })
    );
    for change in ["request", "sender", "loading"] {
        let mut app = board(&[("a", waiting())]);
        keys(&mut app, &[Char('a'), Enter, Char('y')]);
        match change {
            "request" => {
                app.view.as_mut().unwrap().home.as_mut().unwrap().sections[0].rows[0].member["waitingOnYou"] =
                    json!([])
            }
            "sender" => app.view.as_mut().unwrap().me = Some("other".into()),
            _ => app.current = Some("a".into()),
        }
        assert_eq!(press(&mut app, Enter), Effect::None);
        assert!(app.notice.unwrap().contains("nothing sent"));
    }
    let mut app = board(&[("a", waiting())]);
    keys(&mut app, &[Char('a'), Esc]);
    assert!(app.input.is_none() && app.menu.is_none());
    press(&mut app, Char('a'));
    press(&mut app, Enter);
    assert_eq!(press(&mut app, Enter), Effect::None);
    assert_eq!(app.notice.as_deref(), Some("Nothing sent."));
}

#[test]
fn pending_and_squad_notes_use_the_actual_lead_and_refuse_changes() {
    let mut doc = waiting();
    doc["sections"][0]["rows"][0]["waitingOnYou"] = json!([]);
    doc["sections"][0]["rows"][0]["pending"] = json!("note");
    let mut app = board(&[("a", doc.clone())]);
    keys(&mut app, &[Char('a'), Char('x')]);
    assert_eq!(
        press(&mut app, Enter),
        Effect::Act(Request::Annotate {
            me: "ben".into(),
            squad: "a".into(),
            to: "lead-a".into(),
            row: "worker".into(),
            text: "x".into()
        })
    );
    let mut app = board(&[("a", doc)]);
    keys(&mut app, &[Tab, Char('a')]);
    app.view.as_mut().unwrap().home.as_mut().unwrap().squads[0].lead =
        Some(row("NL", "new-lead", "working"));
    press(&mut app, Char('x'));
    assert_eq!(press(&mut app, Enter), Effect::None);
    let mut app = board(&[("a", document("a", Value::Null, vec![]))]);
    press(&mut app, Char('a'));
    assert!(app.input.is_none());
    assert!(app.notice.as_ref().unwrap().contains("no lead"));
    app.view.as_mut().unwrap().me = None;
    press(&mut app, Char('a'));
    assert!(app.notice.as_ref().unwrap().contains("Who is sending"));
    app.finished(Err("send failed".into()));
    assert_eq!(app.notice.as_deref(), Some("send failed"));
}

fn snapshots() -> Value {
    let mut frames = Vec::new();
    for scenario in ["quiet", "waiting", "blocked", "many-squads"] {
        let mut doc = waiting();
        if scenario != "waiting" {
            doc["sections"][0]["rows"][0]["waitingOnYou"] = json!([]);
        }
        if scenario == "quiet" {
            doc["sections"][0]["rows"][0]["state"] = json!("idle");
        }
        let many = (0..14)
            .map(|i| {
                let name = format!("squad-{i:02}");
                (
                    name.clone(),
                    document(
                        &name,
                        row(&format!("L{i}"), &format!("lead-{i}"), "working"),
                        vec![],
                    ),
                )
            })
            .collect::<Vec<_>>();
        let app = if scenario == "many-squads" {
            board(
                &many
                    .iter()
                    .map(|(name, doc)| (name.as_str(), doc.clone()))
                    .collect::<Vec<_>>(),
            )
        } else {
            board(&[("a", doc)])
        };
        for width in [160, 100, 80] {
            app.hits.borrow_mut().clear();
            app.scrolls.begin_frame();
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            terminal
                .draw(|frame| {
                    frame.render_widget(
                        Paragraph::new(paint::summary(
                            app.view.as_ref().unwrap().home.as_ref().unwrap(),
                            width,
                            app.look(),
                        )),
                        Rect::new(0, 0, width, 1),
                    );
                    paint::render_at(frame, &app, Rect::new(0, 1, width, 22), 100);
                    frame.render_widget(
                        Paragraph::new(paint::hints(width as usize, false)),
                        Rect::new(0, 23, width, 1),
                    );
                })
                .unwrap();
            let lines = terminal
                .backend()
                .buffer()
                .content
                .chunks(width as usize)
                .map(|cells| {
                    cells
                        .iter()
                        .map(|cell| cell.symbol())
                        .collect::<String>()
                        .trim_end()
                        .to_owned()
                })
                .collect::<Vec<_>>();
            assert!(!lines.iter().any(|line| line.contains("private question")));
            assert!(scenario != "quiet" || lines.join("").matches("lead-a").count() == 1);
            assert!(
                lines.last().unwrap().contains("? more")
                    && lines.last().unwrap().contains("q quit")
            );
            frames.push(json!({"scenario":scenario,"width":width,"lines":lines,"hits":format!("{:?}",app.hits.borrow())}));
        }
    }
    json!(frames)
}
#[test]
fn home_width_snapshots_cover_quiet_waiting_blocked_and_many_squads() {
    assert_eq!(
        snapshots(),
        serde_json::from_str::<Value>(include_str!("snapshots.json")).unwrap()
    );
}
#[test]
#[ignore = "explicit initial home captures; frozen parity is separate"]
fn record_home_snapshots() {
    fs::write(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/board/home/tests/snapshots.json"
        ),
        serde_json::to_string_pretty(&snapshots()).unwrap() + "\n",
    )
    .unwrap();
}

#[test]
fn middle_home_row_composes_inline_and_success_survives_answer_refresh() {
    for width in [160, 100, 80] {
        for (base, depth) in [
            ("tmt", tmt_cli_style::Depth::TrueColor),
            ("tmt-light", tmt_cli_style::Depth::TrueColor),
            ("tmt", tmt_cli_style::Depth::None),
        ] {
            let members = (0..5).map(|index| {
                let mut member = row(&format!("W{index}"), &format!("worker-{index}"), "working");
                member["waitingOnYou"] = json!([{"requestId":format!("q{index}"), "preview":"Should this decision proceed?", "preparedAtMs":20}]);
                member
            }).collect::<Vec<_>>();
            let doc = document("a", row("L", "lead-a", "working"), members);
            let mut app = board(&[("a", doc.clone())]);
            app.view.as_mut().unwrap().look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(tmt_cli_style::theme::Base::parse(base).unwrap()),
                depth,
            };
            press(&mut app, Down);
            press(&mut app, Down);
            press(&mut app, Char('a'));
            assert!(app.menu.is_none());
            let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let band = app.input_band.get().unwrap();
            let lines = terminal
                .backend()
                .buffer()
                .content
                .chunks(usize::from(width))
                .map(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>())
                .collect::<Vec<_>>();
            assert!(lines[usize::from(band.y + 1)].contains("→ worker-2 (a)"));
            assert!(lines[usize::from(band.y + 2)].contains("◆ “Should this decision proceed?”"));
            assert!(lines[usize::from(band.y + 4)].contains("Enter send · Esc cancel"));
            assert!(lines[usize::from(band.bottom())].contains("worker-3"));
            assert!(
                !app.hits
                    .borrow()
                    .iter()
                    .any(|hit| (band.y..band.bottom()).contains(&hit.y))
            );
            press(&mut app, Tab);
            assert_eq!(app.input.as_ref().unwrap().header(), "✎ note → lead-a");
            press(&mut app, Tab);
            press(&mut app, Char('y'));
            assert!(
                matches!(press(&mut app,Enter),Effect::Act(Request::Reply { ref request,.. }) if request == "q2")
            );
            app.finished(Ok("Replied".into()));
            let mut answered = doc;
            answered["sections"][0]["rows"][2]["waitingOnYou"] = json!([]);
            let refreshed = board(&[("a", answered)]);
            app.apply(Snapshot {
                squad_keys: vec!["a".into()],
                tabs: app.tabs.clone(),
                hidden: vec![],
                pinned: 0,
                attention: Default::default(),
                squad: Some(ALL.into()),
                view: Ok(refreshed.view.unwrap()),
            });
            assert_eq!(app.selected_row().unwrap()["name"], "worker-2");
            assert_eq!(app.selected, 2);
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(text.contains("✓ sent"));
            press(&mut app, Char('x'));
            assert!(app.sent.is_none());
            assert!(!app.home_entries().iter().any(
                |entry| entry.row["name"] == "worker-2" && entry.target.section == "needs-you"
            ));
        }
    }
}
