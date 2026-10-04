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

fn press(app: &mut App, key: KeyCode) -> Effect {
    app.key(KeyEvent::new(key, KeyModifiers::NONE))
}
fn board(documents: &[(&str, Value)]) -> App {
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
fn keys(app: &mut App, codes: &[KeyCode]) -> Effect {
    codes.iter().fold(Effect::None, |_, code| press(app, *code))
}

fn waiting() -> Value {
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
                        Paragraph::new(paint::hints(width as usize)),
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

fn tile_board() -> App {
    let names = ["a", "b", "c", "d", "e"];
    board(&names.map(|name| {
        (
            name,
            document(
                name,
                row(&format!("L{name}"), &format!("lead-{name}"), "working"),
                vec![row(&format!("W{name}"), "worker", "idle")],
            ),
        )
    }))
}

fn tile_frame(app: &App, area: Rect) -> ratatui::buffer::Buffer {
    app.hits.borrow_mut().clear();
    app.scrolls.begin_frame();
    let mut terminal =
        Terminal::new(TestBackend::new(area.right() + 1, area.bottom() + 1)).unwrap();
    terminal
        .draw(|frame| paint::render_at(frame, app, area, 100))
        .unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn tile_continuations_click_the_same_stable_squad_and_gaps_have_no_hits() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    for width in [160, 100, 80] {
        let mut app = tile_board();
        let area = Rect::new(2, 1, width, 24);
        tile_frame(&app, area);
        let hits = app.hits.borrow().clone();
        let height = if width < 100 { 1 } else { 3 };
        for row in 0..5 {
            let tiles = hits.iter().filter(|hit| hit.row == row).collect::<Vec<_>>();
            assert_eq!(tiles.len(), height);
            assert!(tiles.windows(2).all(|pair| pair[1].y == pair[0].y + 1));
        }
        for hit in &hits {
            assert!((area.x..area.right()).contains(&hit.x));
            assert!(hit.x + hit.width <= area.right());
            assert!((area.y..area.bottom()).contains(&hit.y));
        }
        if width >= 100 {
            let left = hits.iter().find(|hit| hit.row == 0).unwrap();
            assert!(!hits.iter().any(|hit| hit.y == left.y
                && (hit.x..hit.x + hit.width).contains(&(left.x + left.width))));
        }
        let continuation = hits.iter().rfind(|hit| hit.row == 4).unwrap();
        assert_eq!(
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: continuation.x + 1,
                    row: continuation.y,
                    modifiers: KeyModifiers::NONE
                },
                std::time::Instant::now()
            ),
            Effect::None
        );
        assert_eq!(app.selected, 4);
        assert_eq!(app.home_target.as_ref().unwrap().squad, "e");
        assert!(app.home_target.as_ref().unwrap().member.is_none());
        assert_eq!(press(&mut app, Enter), Effect::Load("e".into()));
    }
}

#[test]
fn full_tile_reveal_and_clipped_continuation_hits_share_the_scroll_viewport() {
    use crate::{board::scroll::Step, config::Pane};
    let mut app = tile_board();
    app.select(4);
    let target = app.home_target.clone();
    for width in [160, 100, 80, 160] {
        let area = Rect::new(3, 2, width, 5);
        tile_frame(&app, area);
        let selected = app
            .hits
            .borrow()
            .iter()
            .filter(|hit| hit.row == 4)
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), if width < 100 { 1 } else { 3 });
        assert!(
            selected
                .iter()
                .all(|hit| hit.y >= area.y && hit.y < area.bottom() - 1)
        );
        assert_eq!(app.home_target, target);
    }
    // A viewport shorter than a tile clips hits to visible continuations.
    app.follow = false;
    let area = Rect::new(3, 2, 100, 3);
    tile_frame(&app, area);
    app.scrolls.scroll(Pane::Rows, Step::Bottom);
    tile_frame(&app, area);
    let hits = app.hits.borrow().clone();
    assert_eq!(hits.iter().filter(|hit| hit.row == 4).count(), 2);
    assert!(hits.iter().all(|hit| hit.y < area.bottom() - 1));
    assert_eq!(app.selected, 4);
    app.follow = true;
    tile_frame(&app, Rect::new(3, 2, 100, 5));
    assert_eq!(
        app.hits.borrow().iter().filter(|hit| hit.row == 4).count(),
        3
    );
}
