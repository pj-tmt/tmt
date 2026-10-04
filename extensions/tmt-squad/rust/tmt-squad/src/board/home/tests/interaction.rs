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
use tmt_cli_style::breakpoint::{LG, MD};

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
    assert_eq!(app.home_target.as_ref().unwrap().section, "leads");
    assert_eq!(
        press(&mut app, Enter),
        Effect::Act(Request::Jump("lead-a".into()))
    );
    press(&mut app, Down);
    assert_eq!(app.home_target.as_ref().unwrap().section, "all-leads");
    press(&mut app, Down);
    assert_eq!(app.home_target.as_ref().unwrap().section, "squads");
    assert_eq!(press(&mut app, Enter), Effect::Load("a".into()));
    let mut app = board(&[("a", waiting())]);
    let selected = app.home_target.clone();
    for key in [Tab, Tab, BackTab, BackTab] {
        assert_eq!(press(&mut app, key), Effect::None);
        assert_eq!(
            app.home_target, selected,
            "Tab uses board focus, not row navigation"
        );
    }
    assert!(!paint::hints(200, true).contains("tab section"));
    let navigation = crate::board::help::model(&app).sections.remove(0);
    assert!(
        navigation
            .entries
            .iter()
            .all(|entry| !entry.description.contains("previous section"))
    );
    assert!(
        navigation
            .entries
            .iter()
            .any(|entry| entry.keys == "↑↓ / j k" && entry.description.contains("cron"))
    );
    press(&mut app, Down);
    assert_eq!(app.home_target.as_ref().unwrap().section, "blocked");
    press(&mut app, Up);
    assert_eq!(app.home_target, selected);
    assert_eq!(
        press(&mut app, Enter),
        Effect::Act(Request::Jump("worker".into()))
    );

    app.view
        .as_mut()
        .unwrap()
        .bindings
        .insert("tab".into(), Action::parse("refresh").unwrap());
    assert_eq!(
        press(&mut app, Tab),
        Effect::Refresh,
        "explicit board binding still wins"
    );
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
    let view = app.view.as_ref().unwrap();
    app.home_leads
        .reconcile(view.home.as_ref().unwrap(), view.me_id.as_deref());
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
    keys(&mut app, &[End, Char('a')]);
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
            assert!(scenario != "quiet" || lines.join("").matches("lead-a").count() == 2);
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
            assert_eq!(
                app.input.as_ref().unwrap().header(),
                "✎ note → lead-a · about worker-2"
            );
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

fn tile_board() -> App {
    tile_board_with(["a", "b", "c", "d", "e"])
}

fn tile_board_with<const N: usize>(names: [&str; N]) -> App {
    let mut app = board(&names.map(|name| {
        (
            name,
            document(
                name,
                row(&format!("L{name}"), &format!("lead-{name}"), "working"),
                vec![row(&format!("W{name}"), "worker", "idle")],
            ),
        )
    }));
    // These cases isolate the squad table's own geometry; combined HOME lead
    // selection, clipping and bands are covered in leads.rs.
    app.home_leads.leads.clear();
    app.select(0);
    app
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
fn table_rows_click_the_same_stable_squad_at_every_width() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    for width in [160, 100, 80] {
        let mut app = tile_board();
        let area = Rect::new(2, 1, width, 24);
        tile_frame(&app, area);
        let hits = app.hits.borrow().clone();
        let height = 1;
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
fn table_row_reveal_and_hits_share_the_scroll_viewport() {
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
        assert_eq!(selected.len(), 1);
        assert!(
            selected
                .iter()
                .all(|hit| hit.y >= area.y && hit.y < area.bottom() - 1)
        );
        assert_eq!(app.home_target, target);
    }
    // A short viewport keeps one clipped row hit with a stable target.
    app.follow = false;
    let area = Rect::new(3, 2, 100, 3);
    tile_frame(&app, area);
    app.scrolls.scroll(Pane::Rows, Step::Bottom);
    tile_frame(&app, area);
    let hits = app.hits.borrow().clone();
    assert_eq!(hits.iter().filter(|hit| hit.row == 4).count(), 1);
    assert!(hits.iter().all(|hit| hit.y < area.bottom() - 1));
    assert_eq!(app.selected, 4);
    app.follow = true;
    tile_frame(&app, Rect::new(3, 2, 100, 5));
    assert_eq!(
        app.hits.borrow().iter().filter(|hit| hit.row == 4).count(),
        1
    );
}

#[test]
fn table_note_band_shifts_later_rows_without_changing_hits_or_selection() {
    for width in [160, 100, 80] {
        for (base, depth) in [
            ("tmt", tmt_cli_style::Depth::TrueColor),
            ("tmt-light", tmt_cli_style::Depth::TrueColor),
            ("tmt", tmt_cli_style::Depth::None),
        ] {
            let mut app = tile_board_with(["a", "b", "c", "d", "e", "f", "g", "h", "i"]);
            app.view.as_mut().unwrap().look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(tmt_cli_style::theme::Base::parse(base).unwrap()),
                depth,
            };
            app.select(4);
            let target = app.home_target.clone();
            press(&mut app, Char('a'));
            let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let band = app
                .input_band
                .get()
                .expect("note beneath the selected tile");
            let hits = app.hits.borrow().clone();
            let selected = hits.iter().filter(|hit| hit.row == 4).collect::<Vec<_>>();
            assert_eq!(selected.len(), 1);
            assert_eq!(selected.last().unwrap().y + 1, band.y);
            assert!(
                !hits
                    .iter()
                    .any(|hit| (band.y..band.bottom()).contains(&hit.y))
            );
            let next_grid_row = 5;
            assert!(hits.iter().any(|hit| hit.row == next_grid_row));
            assert!(
                hits.iter()
                    .filter(|hit| hit.row == 0)
                    .all(|hit| hit.y < band.y)
            );
            assert!(
                hits.iter()
                    .filter(|hit| hit.row == next_grid_row)
                    .all(|hit| hit.y >= band.bottom())
            );
            let lines = terminal
                .backend()
                .buffer()
                .content
                .chunks(width as usize)
                .map(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>())
                .collect::<Vec<_>>();
            assert!(lines[usize::from(band.y + 1)].contains("✎ note → lead-e · about e"));
            press(&mut app, Esc);
            assert!(app.input.is_none());
            assert_eq!(app.home_target, target);
            press(&mut app, Char('a'));
            press(&mut app, Char('x'));
            assert!(
                matches!(press(&mut app, Enter), Effect::Act(Request::Annotate { ref to, .. }) if to == "lead-e")
            );
            app.finished(Ok("Sent".into()));
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            assert!(
                terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
                    .contains("✓ sent")
            );
            assert_eq!(app.home_target, target);
        }
    }
}

#[test]
fn home_tiles_paint_uncovered_known_history_as_partial_at_each_width_and_theme() {
    use crate::board::{
        app::RateView,
        rate::tests::{fixture_seeds, historical_input, history_fixture},
    };
    use crate::config::TokenRate;
    let fixture = history_fixture();
    let input = historical_input(&fixture, false);
    let mut seeds = fixture_seeds(&fixture);
    let offset = crate::status::now_ms() / 5_000 * 5_000 - 25_000;
    let seed = seeds.get_mut("a").unwrap().as_mut().unwrap();
    seed.from += offset;
    seed.through += offset;
    seed.available = seed.available.map(|at| at + offset);
    seed.sampled = seed.sampled.map(|at| at + offset);
    for bucket in &mut seed.buckets {
        bucket.from_ms += offset;
        bucket.to_ms += offset;
        bucket.covered_ms = 0;
        bucket.complete = false;
        bucket.gap = true;
    }
    for width in [160, 100, 80] {
        for (base, depth) in [
            ("tmt", tmt_cli_style::Depth::TrueColor),
            ("tmt-light", tmt_cli_style::Depth::TrueColor),
            ("tmt", tmt_cli_style::Depth::None),
        ] {
            let mut app = board(&[(
                "a",
                document("a", row("a", "product-lead", "working"), vec![]),
            )]);
            let settings = TokenRate {
                enabled: true,
                ..Default::default()
            };
            let view = app.view.as_mut().unwrap();
            view.look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::parse(base).unwrap()),
                depth,
            };
            view.home_rate.insert(
                "a".into(),
                RateView {
                    input: input.clone(),
                    settings,
                    history: Some(seeds.clone()),
                },
            );
            let mut snapshot = crate::board::app::tests::snapshot(ALL, json!([]));
            snapshot.tabs = app.tabs.clone();
            snapshot.view = Ok(app.view.take().unwrap());
            app.apply(snapshot);
            let now = std::time::Instant::now();
            let usage = app.home_usage("a", now).unwrap();
            assert_eq!(usage.lead[2].unwrap().tokens, 21);
            assert!(usage.share.unwrap().partial);
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let screen = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(screen.contains("product-lead"), "{base}/{width}: {screen}");
            let header = terminal
                .backend()
                .buffer()
                .content
                .chunks(width as usize)
                .nth(2)
                .unwrap()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            if width >= MD.cells {
                assert!(
                    header.starts_with("tok ") && header.contains("share 1h: "),
                    "{base}/{width}: {header}"
                );
                assert_eq!(header.contains("1m ~21"), width >= LG.cells);
                assert_eq!(header.contains("models "), width >= LG.cells);
            } else {
                assert!(!header.starts_with("tok "), "{base}/{width}: {header}");
            }
            assert!(
                screen.matches("~21").count() >= 2,
                "{base}/{width}: {screen}"
            );
            if width >= 100 {
                assert!(screen.contains("~100%"), "{base}/{width}: {screen}");
            }
        }
    }
}

fn header_usage() -> crate::board::app::HomeHeaderUsage<'static> {
    use crate::board::{
        app::{HomeHeaderUsage, UsageModel, UsageShare, UsageTop},
        rate::Reading,
    };
    HomeHeaderUsage {
        windows: crate::config::TokenWindow::DEFAULTS,
        totals: [
            None,
            Some(Reading {
                tokens: 1_200,
                partial: true,
                span: 20_000,
            }),
            Some(Reading {
                tokens: 2_000,
                partial: true,
                span: 20_000,
            }),
        ],
        top: Some(UsageTop {
            member: "worker",
            share: UsageShare {
                fraction: 0.6,
                partial: true,
            },
        }),
        models: vec![
            UsageModel {
                model: Some("gpt-6.1-sol"),
                share: UsageShare {
                    fraction: 0.6,
                    partial: true,
                },
            },
            UsageModel {
                model: Some("claude-opus-4-6"),
                share: UsageShare {
                    fraction: 0.4,
                    partial: true,
                },
            },
        ],
        unreported: 2,
    }
}

#[test]
fn header_usage_formats_thresholds_real_labels_and_partial_missing_values() {
    let app = board(&[]);
    let mut usage = header_usage();
    assert!(paint::usage(&usage, MD.cells - 1, app.look()).is_none());
    for width in [MD.cells, LG.cells - 1] {
        let line = paint::usage(&usage, width, app.look()).unwrap();
        assert_eq!(super::glyph_error(&line.to_string()), None);
        assert_eq!(
            line.to_string(),
            "tok 5m ~1k · 1h ~2k · share 1h: worker ~60%"
        );
    }
    for width in [LG.cells, 149, 160] {
        let line = paint::usage(&usage, width, app.look()).unwrap();
        assert_eq!(super::glyph_error(&line.to_string()), None);
        assert_eq!(
            line.to_string(),
            "tok 1m – · 5m ~1k · 1h ~2k · share 1h: worker ~60% · models sol ~60%, opus ~40% · 2 members without data"
        );
        assert!(line.width() <= width as usize);
    }
    usage.windows =
        ["5m", "1h", "24h"].map(|text| crate::config::TokenWindow::parse(text).unwrap());
    let line = paint::usage(&usage, 100, app.look()).unwrap().to_string();
    assert!(line.starts_with("tok 1h ~1k · 24h ~2k · share 24h:") && line.ends_with("~60%"));
    usage.unreported = 1;
    let line = paint::usage(&usage, LG.cells, app.look())
        .unwrap()
        .to_string();
    assert!(line.ends_with("1 member without data"));
    usage.totals = [None; 3];
    assert!(
        paint::usage(&usage, 160, app.look()).is_none(),
        "no observed samples admit no header row"
    );
    usage.totals[2] = Some(crate::board::rate::Reading {
        tokens: 0,
        partial: false,
        span: 3_600_000,
    });
    usage.top = None;
    usage.models.clear();
    let line = paint::usage(&usage, 160, app.look()).unwrap().to_string();
    assert!(line.contains("24h 0 · share 24h: – · models – · 1 member without data"));
}

#[test]
fn header_usage_fits_escaped_unicode_names_and_keeps_whole_optional_groups() {
    let app = board(&[]);
    let mut usage = header_usage();
    usage.top.as_mut().unwrap().member = "long-界界界界界界-e\u{301}-worker\nunsafe";
    usage.models[0].model = Some("unknown-model-with-a-very-long-name\u{1b}");
    for width in [MD.cells, LG.cells - 1, LG.cells, 149, 160] {
        let line = paint::usage(&usage, width, app.look()).unwrap();
        let text = line.to_string();
        assert!(line.width() <= width as usize, "{width}: {text}");
        assert!(!text.contains('\n') && !text.contains('\u{1b}'));
        assert!(text.contains("share 1h:") && text.contains("~60%"));
        assert_eq!(text.contains("2 members without data"), width >= LG.cells);
        assert!(!text.ends_with(" · "));
    }
}

fn header_frames() -> Value {
    let mut captures = Vec::new();
    for (base, depth, name) in [
        ("tmt", tmt_cli_style::Depth::TrueColor, "tmt"),
        ("tmt-light", tmt_cli_style::Depth::TrueColor, "tmt-light"),
        ("tmt", tmt_cli_style::Depth::None, "NO_COLOR"),
    ] {
        let mut app = board(&[
            ("a", document("a", row("A", "lead-a", "working"), vec![])),
            ("b", document("b", row("B", "lead-b", "working"), vec![])),
        ]);
        app.view.as_mut().unwrap().look = crate::look::Look {
            theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::parse(base).unwrap()),
            depth,
        };
        for width in [160, 100, 80, 160] {
            let mut frames = Vec::new();
            let mut hits = Vec::new();
            let mut buffers = Vec::new();
            for show in [false, true] {
                let line = show
                    .then(|| paint::usage(&header_usage(), width, app.look()))
                    .flatten();
                let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
                terminal
                    .draw(|frame| crate::board::view::render_frame(frame, &app, line))
                    .unwrap();
                let buffer = terminal.backend().buffer().clone();
                let cells = buffer
                    .content
                    .iter()
                    .map(|cell| {
                        let mut metadata = cell.clone();
                        metadata.set_symbol("");
                        json!({"symbol":cell.symbol(), "style":format!("{metadata:?}")})
                    })
                    .collect::<Vec<_>>();
                let hit = app.hits.borrow().clone();
                frames.push(json!({"cells":cells,"hits":format!("{hit:?}"),
                    "starts":*app.row_starts.borrow(), "tabs":format!("{:?}",app.tab_hits.borrow())}));
                hits.push(hit);
                buffers.push(buffer);
            }
            let shift = u16::from(width >= MD.cells);
            assert_eq!(hits[0].len(), hits[1].len());
            for (before, after) in hits[0].iter().zip(&hits[1]) {
                assert_eq!(after.y, before.y + shift);
                assert_eq!(
                    (after.x, after.width, after.row),
                    (before.x, before.width, before.row)
                );
            }
            if shift == 0 {
                assert_eq!(frames[0], frames[1]);
            } else {
                assert_eq!(
                    &buffers[0].content[..2 * width as usize],
                    &buffers[1].content[..2 * width as usize]
                );
                for y in 2..28 {
                    let start = y * width as usize;
                    let next = start + width as usize;
                    assert_eq!(
                        &buffers[0].content[start..next],
                        &buffers[1].content[next..next + width as usize]
                    );
                }
            }
            captures.push(json!({"theme":name,"width":width,"before":frames[0],"after":frames[1]}));
        }
    }
    json!(captures)
}

#[test]
fn home_header_row_preserves_cell_styles_hit_identity_and_resize_order() {
    let frames = header_frames();
    for theme in 0..3 {
        assert_eq!(
            frames[theme * 4],
            frames[theme * 4 + 3],
            "160→100→80→160 restores cells/styles/hits"
        );
    }
}

#[test]
#[ignore = "explicit decoded header evidence, never fixture regeneration"]
fn record_home_header_diff() {
    let path = std::env::var("TMT_HEADER_DIFF_PATH").expect("task-owned evidence output path");
    assert!(std::path::Path::new(&path).starts_with("/private/tmp"));
    fs::write(path, serde_json::to_string(&header_frames()).unwrap()).unwrap();
}
