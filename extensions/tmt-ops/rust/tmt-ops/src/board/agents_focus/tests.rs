use super::*;
use crate::board::{
    app::{
        Effect, Request,
        tests::configure_workspace,
        workspace::tests::{MEMBER, ROOM, board},
    },
    scroll::Step,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};
use serde_json::json;

fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}
fn screen(buffer: &Buffer) -> String {
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
fn observed(body: &str) -> App {
    let mut app = board();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .squad_looks
        .clear();
    app.view.as_mut().unwrap().look = Look {
        theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::Tmt),
        depth: tmt_cli_style::Depth::TrueColor,
    };
    app.view.as_mut().unwrap().replies = vec![json!({"recipientId":MEMBER,
        "requestId":"req_44444444-4444-4444-8444-444444444444", "response":body})];
    app
}

#[test]
fn sparse_exchange_grows_down_and_overflow_alone_pins_the_same_input() {
    let mut app = observed("One short observed response.");
    let target = app.workspace.selected.clone().unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .rows
        .get_mut(&target)
        .unwrap()["pane"]["target"] = json!("work:2.1");
    let sparse = draw(&mut app, 148, 51);
    let text = screen(&sparse);
    for value in [
        "Exchanges with you",
        "One short observed response.",
        "tmux work:2.1",
        "TOK 1h",
        "STATE",
        "DIGEST",
        "TIMERS",
    ] {
        assert!(text.contains(value), "{value}:\n{text}");
    }
    assert!(!text.contains("all parties") && !text.contains("since "));
    let input = app.input_band.get().expect("admitted idle input");
    assert_eq!(input.height, 3);
    assert!(
        input.bottom() < app.conversation.borrow().pane.unwrap().bottom() - 5,
        "sparse input follows content"
    );
    assert_eq!(input.x, 72);
    let fill = crate::board::input_field::style(app.look(), false);
    for y in input.y..input.bottom() {
        for x in input.x..input.right() {
            assert_eq!(sparse[(x, y)].bg, fill.bg.unwrap());
        }
    }
    let mut long = observed(
        &(0..120)
            .map(|n| format!("Observed line {n:03}  \n"))
            .collect::<String>(),
    );
    let text = screen(&draw(&mut long, 148, 51));
    let band = long.input_band.get().unwrap();
    assert_eq!(
        band.bottom(),
        long.conversation.borrow().pane.unwrap().bottom()
    );
    assert!(
        text.contains("earlier") && text.contains("Observed line 119"),
        "{text}"
    );
    long.scroll_pane(Pane::Replies, Step::Top);
    let top = screen(&draw(&mut long, 148, 51));
    assert!(top.contains("Observed line 000"), "{top}");
    assert_eq!(long.input_band.get(), Some(band));
}

#[test]
fn squad_actions_follow_only_the_verified_current_lead_from_either_pane() {
    for focus in [0, 1] {
        let mut app = observed("Lead reply.");
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
        app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
        app.workspace_reconcile();
        app.focus = focus;
        let text = screen(&draw(&mut app, 148, 51));
        for value in [
            "lead worker",
            "Board",
            "Usage",
            "Timers",
            "Exchanges with you",
            "Lead reply.",
        ] {
            assert!(text.contains(value), "{value}:\n{text}");
        }
        assert!(app.composer_start().is_some());
        assert_eq!(
            app.workspace_bindings()["j"].verb,
            crate::action::Verb::Jump
        );
        assert_eq!(
            app.workspace_bindings()["tab"].verb,
            crate::action::Verb::NextPane
        );
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            app.input
                .as_ref()
                .unwrap()
                .row_send
                .as_ref()
                .is_some_and(|send| send.target
                    == RowTarget::Workspace(OpsTarget::Member {
                        room_id: Some(ROOM.into()),
                        identity_id: MEMBER.into()
                    }))
        );
        app.input = None;
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .roster
            .squads[0]
            .lead = Some("55555555-5555-4555-8555-555555555555".into());
        assert!(app.workspace_message_target().is_none());
        assert!(app.composer_projection().is_none());
    }
}

#[test]
fn authored_reader_and_member_keys_win_over_selected_action_defaults() {
    let mut app = observed("Observed.");
    configure_workspace(
        &mut app,
        "[bind]\nj = 'copy custom'\nenter = 'copy enter'\ntab = 'copy tab'\n",
    );
    for focus in [0, 1] {
        app.focus = focus;
        draw(&mut app, 148, 51);
        for key in ["j", "enter", "tab"] {
            assert_eq!(
                app.workspace_bindings()[key].verb,
                crate::action::Verb::Copy
            );
        }
        assert!(app.composer_start().is_none());
        assert!(matches!(
            app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
            Effect::Act(Request::Copy { .. })
        ));
    }
}

#[test]
fn timer_cells_count_owned_paused_jobs_and_keep_zero_separate_from_unavailable() {
    use crate::board::cronboard::{test_cron, test_views};
    use tmt_ops::cron::{ClockStatus, Pause};
    let mut app = observed("Observed response.");
    let selected = app.workspace.selected.clone().unwrap();
    let mut jobs = test_views(3, "product", ROOM);
    jobs[0].job.owner_id = Some(MEMBER.into());
    jobs[1].job.owner_id = Some(MEMBER.into());
    jobs[1].job.pause = Some(Pause {
        by: "owner".into(),
        at_ms: 1,
    });
    jobs[2].job.owner_id = Some("55555555-5555-4555-8555-555555555555".into());
    let mut other_room = test_views(1, "elsewhere", "66666666-6666-4666-8666-666666666666");
    other_room[0].job.owner_id = Some(MEMBER.into());
    jobs.extend(other_room);
    let line_at = |app: &mut App| {
        let buffer = draw(app, 148, 51);
        let hit = app
            .hits
            .borrow()
            .iter()
            .find(|hit| match hit.target {
                crate::board::app::HitTarget::Row(index) => app
                    .workspace_rows()
                    .get(index)
                    .is_some_and(|row| row.target == selected),
                _ => false,
            })
            .cloned()
            .expect("painted member row");
        buffer[(54, hit.y)].symbol().to_owned()
    };
    assert_eq!(line_at(&mut app), "—");
    app.cron.replace(Ok(test_cron(jobs, ClockStatus::NoClock)));
    assert_eq!(
        line_at(&mut app),
        "2",
        "owned total includes paused, excludes other owner/room"
    );
    app.cron
        .replace(Ok(test_cron(vec![], ClockStatus::NoClock)));
    assert_eq!(line_at(&mut app), "0");
}

#[test]
fn responsive_and_short_focus_keep_an_admitted_editor_and_its_uuid() {
    let mut app = observed("Short response.");
    let selected = app.workspace.selected.clone();
    for (width, height) in [
        (148, 51),
        (103, 51),
        (100, 24),
        (80, 24),
        (60, 18),
        (148, 17),
    ] {
        draw(&mut app, width, height);
        let pane = app.conversation.borrow().pane;
        if let Some(pane) = pane.filter(|pane| pane.height >= 8) {
            let input = app
                .input_band
                .get()
                .expect("usable conversation retains its input");
            assert_eq!(input.height, 3);
            assert_eq!(input.intersection(pane), input);
        }
        assert_eq!(app.workspace.selected, selected);
    }
}

mod captures;

#[test]
fn overflowing_agents_panes_reach_the_painted_footer_in_both_focuses() {
    crate::status::with_now_ms(1_791_600_000_000, || {
        let body = (0..120)
            .map(|n| format!("Observed line {n:03}  \n"))
            .collect::<String>();
        let mut failures = Vec::new();
        let mut frames = Vec::new();
        for (look, base, depth) in [
            (
                "tmt",
                tmt_cli_style::Base::Tmt,
                tmt_cli_style::Depth::TrueColor,
            ),
            (
                "tmt-light",
                tmt_cli_style::Base::TmtLight,
                tmt_cli_style::Depth::TrueColor,
            ),
            (
                "terminal",
                tmt_cli_style::Base::Terminal,
                tmt_cli_style::Depth::Ansi16,
            ),
            (
                "NO_COLOR",
                tmt_cli_style::Base::Tmt,
                tmt_cli_style::Depth::None,
            ),
        ] {
            for width in [103, 148] {
                for height in [50, 51, 53] {
                    for editing in [false, true] {
                        for focus in [0, 1] {
                            let mut app = observed(&body);
                            app.view.as_mut().unwrap().look = Look {
                                theme: tmt_cli_style::Theme::new(base),
                                depth,
                            };
                            if editing {
                                app.perform(&crate::action::Action::parse("talk").unwrap());
                                assert!(app.input.is_some());
                            }
                            app.focus = focus;
                            let _no_acquisition = crate::runner::forbid_commands();
                            let buffer = draw(&mut app, width, height);
                            let text = screen(&buffer);
                            let lines = text.lines().collect::<Vec<_>>();
                            let pane = app.conversation.borrow().pane.unwrap();
                            let footer_rows = if focus == 0 && !editing { 2 } else { 1 };
                            let expected_bottom = height - footer_rows;
                            let context =
                                format!("{look} {width}x{height} focus={focus} editing={editing}");
                            if pane.bottom() != expected_bottom {
                                failures.push(format!(
                                    "{context}: pane ends {}, footer starts {expected_bottom}",
                                    pane.bottom()
                                ));
                            }
                            let band = app.input_band.get().unwrap();
                            if band.bottom() != pane.bottom() {
                                failures.push(format!(
                                    "{context}: overflowing input ends {}, pane ends {}",
                                    band.bottom(),
                                    pane.bottom()
                                ));
                            }
                            for y in expected_bottom..height {
                                if lines[usize::from(y)].trim().is_empty() {
                                    failures.push(format!("{context}: blank footer row {y}"));
                                }
                            }
                            let last_body_y = expected_bottom - 1;
                            let filled_input = band.y <= last_body_y
                                && last_body_y < band.bottom()
                                && (band.x..band.right()).all(|x| {
                                    let fill =
                                        super::super::input_field::style(app.look(), editing);
                                    let cell = &buffer[(x, last_body_y)];
                                    fill.bg.map_or_else(
                                        || cell.modifier.contains(fill.add_modifier),
                                        |bg| cell.bg == bg,
                                    )
                                });
                            if lines[usize::from(last_body_y)].trim().is_empty() && !filled_input {
                                failures.push(format!("{context}: blank band above footer"));
                            }
                            frames.push(json!({"look":look,"width":width,"height":height,"focus":focus,"editing":editing,
                            "pane":format!("{pane:?}"),"input":format!("{band:?}"),
                            "lines":lines,
                            "cells":buffer.content.iter().map(|cell| json!([cell.symbol(),format!("{:?}",cell.fg),format!("{:?}",cell.bg),format!("{:?}",cell.modifier)])).collect::<Vec<_>>()}));
                        }
                    }
                }
            }
        }
        if let Some(path) = std::env::var_os("TMT_AGENTS_BOTTOM_CAPTURE") {
            std::fs::write(
                path,
                serde_json::to_vec(&json!({"frames":frames,"failures":failures})).unwrap(),
            )
            .unwrap();
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    });
}
