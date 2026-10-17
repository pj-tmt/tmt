use super::*;
fn event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
    }
}
fn boundary(id: Id) -> Boundary {
    Boundary {
        id,
        hit: if id.columns() {
            Rect::new(50, 2, 1, 30)
        } else {
            Rect::new(2, 50, 100, 1)
        },
        total: 100,
        before: 50,
        min: 20,
        max: 80,
    }
}
#[test]
fn drag_clamps_both_axes_and_reset_restores_exact_default() {
    for id in Id::ALL {
        let state = State::default();
        let b = boundary(id);
        state.record(b, Rect::new(0, 0, 160, 100));
        let now = Instant::now();
        assert_eq!(
            state.mouse(
                event(MouseEventKind::Down(MouseButton::Left), b.hit.x, b.hit.y),
                now
            ),
            Some(false)
        );
        let (x, y) = if id.columns() {
            (159, b.hit.y)
        } else {
            (b.hit.x, 99)
        };
        assert_eq!(
            state.mouse(event(MouseEventKind::Drag(MouseButton::Left), x, y), now),
            Some(true)
        );
        assert_eq!(state.size(id, 100, 47, 20, 20), 80);
        state.mouse(event(MouseEventKind::Up(MouseButton::Left), 0, 0), now);
        assert_eq!(state.size(id, 100, 47, 20, 20), 20);
        assert!(state.drag.get().is_none());
        assert!(state.reset(id));
        assert_eq!(state.size(id, 100, 47, 20, 20), 47);
    }
}
#[test]
fn ratio_survives_resize_without_saving_the_temporary_clamp() {
    let state = State::default();
    state.save(boundary(Id::AgentsColumns), 80);
    assert_eq!(state.size(Id::AgentsColumns, 200, 94, 36, 36), 160);
    assert_eq!(state.size(Id::AgentsColumns, 80, 37, 36, 36), 44);
    assert_eq!(state.size(Id::AgentsColumns, 200, 94, 36, 36), 160);
    assert_eq!(
        ratios_decode(Some(&ratios_json(&state.ratios()))),
        Some(state.ratios())
    );
    assert_eq!(ratios_decode(None), Some(Ratios::new()));
    for invalid in [
        json!({"unknown":5000}),
        json!({"agents-columns":10001}),
        json!({"agents-columns":-1}),
        json!({"agents-columns":0.5}),
        json!([]),
    ] {
        assert!(ratios_decode(Some(&invalid)).is_none());
    }
}
#[test]
fn invalidated_and_clipped_boundaries_cannot_start_or_continue_drag() {
    let state = State::default();
    let b = boundary(Id::AgentsColumns);
    let now = Instant::now();
    state.record(b, Rect::new(0, 0, 50, 100));
    assert!(state.frame.borrow().is_empty());
    state.record(b, Rect::new(0, 0, 160, 100));
    state.mouse(event(MouseEventKind::Down(MouseButton::Left), 50, 3), now);
    state.invalidate();
    assert!(
        state
            .mouse(event(MouseEventKind::Drag(MouseButton::Left), 90, 3), now)
            .is_none()
    );
    assert!(state.ratios().is_empty());
}
#[test]
fn double_click_requires_same_live_boundary_and_never_uses_row_identity() {
    let state = State::default();
    let b = boundary(Id::AgentsColumns);
    let now = Instant::now();
    state.save(b, 70);
    state.record(b, Rect::new(0, 0, 160, 100));
    state.mouse(event(MouseEventKind::Down(MouseButton::Left), 50, 3), now);
    state.mouse(event(MouseEventKind::Up(MouseButton::Left), 50, 3), now);
    assert!(
        state
            .mouse(
                event(MouseEventKind::Down(MouseButton::Left), 50, 3),
                now + Duration::from_millis(100)
            )
            .unwrap()
    );
    assert!(state.ratios().is_empty());
    assert!(state.drag.get().is_none());
}
#[test]
fn action_grammar_has_explicit_boundary_axis_and_no_implicit_arguments() {
    for id in Id::ALL {
        for direction in if id.columns() {
            ["left", "right", "reset"]
        } else {
            ["up", "down", "reset"]
        } {
            assert!(
                crate::action::Action::parse(&format!("resize-pane {} {direction}", id.name()))
                    .is_ok()
            );
        }
        let wrong = if id.columns() { "down" } else { "left" };
        assert!(
            crate::action::Action::parse(&format!("resize-pane {} {wrong}", id.name())).is_err()
        );
    }
    for action in [
        "resize-pane",
        "resize-pane agents-columns",
        "resize-pane whatever left",
        "resize-pane agents-columns right 10",
    ] {
        assert!(crate::action::Action::parse(action).is_err());
    }
}

fn app(tab: crate::board::shell::OpsTab) -> App {
    let mut app = crate::board::app::workspace::tests::board();
    crate::board::app::workspace::tests::observe_waiting_member(&mut app);
    app.shell.tab = tab;
    app
}
fn paint(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    crate::status::with_now_ms(1_791_600_000_000, || {
        terminal.draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
    })
    .unwrap();
    terminal.backend().buffer().clone()
}
#[test]
fn painted_panes_drag_without_row_effects_and_keyboard_uses_the_same_geometry() {
    use crate::board::shell::OpsTab;
    let _no_commands = crate::runner::forbid_commands();
    for (tab, width, id) in [
        (OpsTab::Agents, 160, Id::AgentsColumns),
        (OpsTab::Agents, 100, Id::AgentsRows),
        (OpsTab::Digest, 160, Id::DigestColumns),
        (OpsTab::Digest, 100, Id::DigestRows),
        (OpsTab::Overview, 160, Id::OverviewColumns),
    ] {
        let mut app = app(tab);
        paint(&mut app, width, 51);
        let b = app
            .pane_resize
            .frame
            .borrow()
            .iter()
            .find(|b| b.id == id)
            .copied()
            .expect("visible split");
        let target = app.workspace.selected.clone();
        let now = Instant::now();
        assert_eq!(
            app.mouse(
                event(MouseEventKind::Down(MouseButton::Left), b.hit.x, b.hit.y),
                now
            ),
            Effect::None
        );
        assert_eq!(
            app.mouse(
                event(MouseEventKind::Up(MouseButton::Left), b.hit.x, b.hit.y),
                now
            ),
            Effect::None
        );
        assert!(
            app.pane_resize.ratios().is_empty(),
            "a plain click preserves the default"
        );
        // Let the independent click sequence expire before beginning a drag.
        let now = now + Duration::from_secs(1);
        app.mouse(
            event(MouseEventKind::Down(MouseButton::Left), b.hit.x, b.hit.y),
            now,
        );
        let (x, y) = if id.columns() {
            (b.hit.x + 5, b.hit.y)
        } else {
            (b.hit.x, b.hit.y + 2)
        };
        assert_eq!(
            app.mouse(event(MouseEventKind::Drag(MouseButton::Left), x, y), now),
            Effect::None
        );
        paint(&mut app, width, 51);
        assert_eq!(
            app.mouse(event(MouseEventKind::Up(MouseButton::Left), x, y), now),
            Effect::None
        );
        paint(&mut app, width, 51);
        assert_eq!(
            app.pane_resize.hint(),
            Some(if id.columns() { "↔" } else { "↕" }),
            "a released drag still hovers its current painted divider"
        );
        assert!(app.move_pointer(event(MouseEventKind::Moved, 0, 0)));
        assert!(
            app.pane_resize.hint().is_none(),
            "leaving the divider clears its cue"
        );
        assert_eq!(app.workspace.selected, target);
        assert!(app.menu.is_none());
        assert!(app.input.is_none());
        let moved = app
            .pane_resize
            .frame
            .borrow()
            .iter()
            .find(|b| b.id == id)
            .copied()
            .unwrap();
        assert!(moved.before > b.before, "{tab:?} {id:?}");
        let action =
            crate::action::Action::parse(&format!("resize-pane {} reset", id.name())).unwrap();
        assert_eq!(app.perform(&action), Effect::None);
        paint(&mut app, width, 51);
        let reset = app
            .pane_resize
            .frame
            .borrow()
            .iter()
            .find(|b| b.id == id)
            .copied()
            .unwrap();
        assert_eq!(reset.before, b.before);
    }
}
#[test]
fn hover_highlights_strokes_and_overlay_or_resize_revokes_drag_authority() {
    use crate::board::shell::OpsTab;
    let mut app = app(OpsTab::Agents);
    let original = paint(&mut app, 160, 51);
    let b = app.pane_resize.frame.borrow()[0];
    assert!(app.move_pointer(event(MouseEventKind::Moved, b.hit.x, b.hit.y + 1)));
    let hovered = paint(&mut app, 160, 51);
    assert_ne!(
        original[(b.hit.x, b.hit.y + 1)].style(),
        hovered[(b.hit.x, b.hit.y + 1)].style()
    );
    assert_eq!(app.pane_resize.hint(), Some("↔"));
    let now = Instant::now();
    app.mouse(
        event(MouseEventKind::Down(MouseButton::Left), b.hit.x, b.hit.y),
        now,
    );
    app.invalidate_frame_geometry();
    app.mouse(
        event(
            MouseEventKind::Drag(MouseButton::Left),
            b.hit.x + 10,
            b.hit.y,
        ),
        now,
    );
    assert!(app.pane_resize.ratios().is_empty());
    paint(&mut app, 160, 51);
    app.help = true;
    paint(&mut app, 160, 51);
    assert!(app.pane_resize.frame.borrow().is_empty());
    assert!(app.pane_resize.hint().is_none());
}
#[test]
fn retained_controls_carry_ratios_but_not_drag_or_hover() {
    use crate::board::{shell::OpsTab, view_state::Controls};
    let first = app(OpsTab::Agents);
    first.pane_resize.restore(
        Id::ALL
            .into_iter()
            .enumerate()
            .map(|(i, id)| (id, 4000 + i as u16 * 100))
            .collect(),
    );
    let controls = Controls::capture(&first);
    let value = controls.json();
    assert_eq!(value["paneRatios"]["agents-columns"], 4100);
    let mut next = app(OpsTab::Agents);
    Controls::decode(&value).unwrap().apply(&mut next);
    assert_eq!(next.pane_resize.ratios(), first.pane_resize.ratios());
    assert!(next.pane_resize.frame.borrow().is_empty());
    assert!(next.pane_resize.drag.get().is_none());
    let mut legacy = value.clone();
    legacy.as_object_mut().unwrap().remove("paneRatios");
    Controls::decode(&legacy).unwrap().apply(&mut next);
    assert!(next.pane_resize.ratios().is_empty());
}
#[test]
fn tiny_one_pane_and_hidden_conversation_have_no_phantom_boundary() {
    use crate::board::shell::OpsTab;
    for tab in [OpsTab::Overview, OpsTab::Agents, OpsTab::Digest] {
        for (width, height) in [(0, 0), (60, 8), (100, 8)] {
            let mut app = app(tab);
            paint(&mut app, width, height);
            assert!(
                app.pane_resize.frame.borrow().is_empty(),
                "{tab:?} {width}x{height}"
            );
        }
    }
    let mut app = app(OpsTab::Overview);
    let mut policy = crate::config::OverviewPanels::default();
    policy.set_side_shown(false).unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .overview_panels = Some(crate::config::PanelSetting::configured(policy));
    paint(&mut app, 160, 51);
    assert!(!app.pane_resize.frame.borrow().iter().any(|b| matches!(
        b.id,
        Id::OverviewColumns | Id::OverviewExchange | Id::OverviewTimers
    )));
}

#[test]
fn overview_section_rules_are_the_only_horizontal_resize_targets() {
    use crate::board::shell::OpsTab;
    for width in [60, 100, 103, 148, 160] {
        let mut app = crate::board::home::tests::oracle::metered(crate::look::Look::default());
        app.shell.tab = OpsTab::Overview;
        paint(&mut app, width, 51);
        let ids: Vec<_> = app
            .pane_resize
            .frame
            .borrow()
            .iter()
            .map(|b| b.id)
            .collect();
        assert!(ids.contains(&Id::OverviewUsage), "{width}: {ids:?}");
        assert!(!ids.contains(&Id::AgentsColumns));
        if width < 140 {
            assert!(ids.contains(&Id::OverviewList), "{width}: {ids:?}");
        }
        let boundaries = app.pane_resize.frame.borrow().clone();
        for b in boundaries {
            if b.id.columns() {
                continue;
            }
            let shown = paint(&mut app, width, 51);
            assert!(
                shown[(b.hit.x, b.hit.y)].symbol().contains('─'),
                "{width}: {:?} is not a section rule",
                b.id
            );
            let before = app.workspace.selected.clone();
            let action =
                crate::action::Action::parse(&format!("resize-pane {} down", b.id.name())).unwrap();
            assert_eq!(app.perform(&action), Effect::None);
            paint(&mut app, width, 51);
            assert_eq!(app.workspace.selected, before);
            let reset = crate::action::Action::parse(&format!("resize-pane {} reset", b.id.name()))
                .unwrap();
            app.perform(&reset);
            paint(&mut app, width, 51);
        }
    }
}

fn populated_app(tab: crate::board::shell::OpsTab, look: crate::look::Look) -> App {
    crate::status::with_now_ms(1_791_600_000_000, || {
        let mut app = crate::board::home::tests::oracle::metered(look);
        crate::board::app::workspace::tests::observe_waiting_member(&mut app);
        app.shell.tab = tab;
        crate::board::overview::capture::populate_lower_band(&mut app, "squads-valued");
        crate::board::overview::capture::populate_final(&mut app, "squads-valued");
        app.workspace_reconcile();
        app
    })
}

#[test]
#[ignore = "exports task-owned native renderer evidence; never records fixtures"]
fn capture_pane_resize_packet() {
    use crate::board::shell::OpsTab;
    use tmt_cli_style::{Depth, Theme, theme::Base};
    let output = std::env::var("TMT_RESIZE_PACKET").expect("task-owned evidence path");
    let mut captures = Vec::new();
    for tab in [OpsTab::Overview, OpsTab::Agents, OpsTab::Digest] {
        for width in [160, 148, 103, 100, 60] {
            for (name, base, depth) in [
                ("tmt", Base::Tmt, Depth::TrueColor),
                ("tmt-light", Base::TmtLight, Depth::TrueColor),
                ("NO_COLOR", Base::Tmt, Depth::None),
            ] {
                let look = crate::look::Look {
                    theme: Theme::new(base),
                    depth,
                };
                let mut app = populated_app(tab, look);
                let mut buffer = paint(&mut app, width, 51);
                for stage in ["before", "after"] {
                    if stage == "after" {
                        let now = Instant::now();
                        // Real pointer events adjust one admitted boundary in each axis.
                        for columns in [true, false] {
                            let current = app
                                .pane_resize
                                .frame
                                .borrow()
                                .iter()
                                .find(|b| b.id.columns() == columns)
                                .copied();
                            if let Some(b) = current {
                                let (x, y) = if columns {
                                    (b.hit.x + 5, b.hit.y)
                                } else {
                                    (b.hit.x, b.hit.y + 2)
                                };
                                assert_eq!(
                                    app.mouse(
                                        event(
                                            MouseEventKind::Down(MouseButton::Left),
                                            b.hit.x,
                                            b.hit.y
                                        ),
                                        now
                                    ),
                                    Effect::None
                                );
                                assert_eq!(
                                    app.mouse(
                                        event(MouseEventKind::Drag(MouseButton::Left), x, y),
                                        now
                                    ),
                                    Effect::None
                                );
                                paint(&mut app, width, 51);
                                assert_eq!(
                                    app.mouse(
                                        event(MouseEventKind::Up(MouseButton::Left), x, y),
                                        now
                                    ),
                                    Effect::None
                                );
                            }
                        }
                        buffer = paint(&mut app, width, 51);
                    }
                    let lines: Vec<String> = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
                        .collect();
                    let cells:Vec<_>=buffer.content.iter().map(|cell|json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect();
                    captures.push(json!({"tab":format!("{tab:?}"),"width":width,"height":51,"look":name,"stage":stage,"lines":lines,"cells":cells,"ratios":ratios_json(&app.pane_resize.ratios()),"boundaries":app.pane_resize.frame.borrow().iter().map(|b|json!({"id":b.id.name(),"x":b.hit.x,"y":b.hit.y,"width":b.hit.width,"height":b.hit.height,"total":b.total,"before":b.before,"min":b.min,"max":b.max})).collect::<Vec<_>>(),"inputBand":format!("{:?}",app.input_band.get())}));
                }
            }
        }
    }
    assert_eq!(captures.len(), 90);
    std::fs::write(output, serde_json::to_vec(&captures).unwrap()).unwrap();
}

#[test]
fn tab_keeps_switching_panes_and_only_an_authored_resize_binding_adjusts_sizes() {
    use crate::board::shell::OpsTab;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = app(OpsTab::Agents);
    paint(&mut app, 160, 51);
    assert!(app.bindings().values().all(
        |action| action.verb != crate::action::Verb::Ui(crate::action::OpsUiAction::ResizePane)
    ));
    let first = app.focused();
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_ne!(app.focused(), first);
    assert!(app.pane_resize.ratios().is_empty());
    app.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(app.focused(), first);
    crate::board::app::tests::configure_workspace(
        &mut app,
        "[bind]\nr='resize-pane agents-columns right'\n",
    );
    paint(&mut app, 160, 51);
    app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    assert!(app.pane_resize.ratios().contains_key(&Id::AgentsColumns));
    crate::board::app::tests::configure_workspace(&mut app, "[bind]\nr='menu'\n");
    paint(&mut app, 160, 51);
    let ratios = app.pane_resize.ratios();
    app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    assert!(app.menu.is_some());
    assert_eq!(app.pane_resize.ratios(), ratios);
}

#[test]
fn growing_usage_never_hides_the_existing_list_timers_or_digest() {
    use crate::board::shell::OpsTab;
    for width in [60, 100, 103, 148, 160] {
        let mut app = crate::board::home::tests::oracle::metered(crate::look::Look::default());
        app.shell.tab = OpsTab::Overview;
        let original = paint(&mut app, width, 51);
        let text = |buffer: &Buffer| {
            buffer
                .content
                .chunks(usize::from(width))
                .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(text(&original).contains("── Digest"));
        app.pane_resize
            .restore([(Id::OverviewUsage, 10_000)].into());
        let clamped = paint(&mut app, width, 51);
        let shown = text(&clamped);
        assert!(
            shown.contains("── Timers") && shown.contains("── Digest"),
            "{width}\n{shown}"
        );
        assert!(app.scrolls.visible(crate::config::Pane::Rows));
        assert_eq!(
            app.pane_resize.ratios()[&Id::OverviewUsage],
            10_000,
            "clamping is not publication"
        );
    }
}

#[test]
fn an_active_drag_defers_exec_reload_until_release() {
    use crate::board::shell::OpsTab;
    let mut app = app(OpsTab::Agents);
    paint(&mut app, 160, 51);
    assert!(!app.reload_blocked());
    let b = app.pane_resize.frame.borrow()[0];
    let now = Instant::now();
    app.mouse(
        event(MouseEventKind::Down(MouseButton::Left), b.hit.x, b.hit.y),
        now,
    );
    assert!(app.reload_blocked());
    app.mouse(
        event(MouseEventKind::Up(MouseButton::Left), b.hit.x, b.hit.y),
        now,
    );
    assert!(!app.reload_blocked());
}

#[test]
fn resize_footer_keeps_authored_help_quit_and_migration_notice_priority() {
    use crate::board::shell::OpsTab;
    let mut app = app(OpsTab::Agents);
    crate::board::app::tests::configure_workspace(
        &mut app,
        "[bind]\n'?'='menu'\nq='menu'\nx='help'\nz='quit'\n",
    );
    paint(&mut app, 160, 51);
    let b = app.pane_resize.frame.borrow()[0];
    app.move_pointer(event(MouseEventKind::Moved, b.hit.x, b.hit.y + 1));
    let buffer = paint(&mut app, 160, 51);
    let footer = |buffer: &Buffer| {
        buffer
            .content
            .chunks(160)
            .skip(48)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let shown = footer(&buffer);
    assert!(
        shown.contains("↔ drag to resize") && shown.contains("x help") && shown.contains("z quit"),
        "{shown}"
    );
    assert_eq!(
        buffer[(0, 48)].symbol(),
        " ",
        "the shared footer keeps its one-cell gutter"
    );
    app.migration_notice = Some("Retired binding requires migration.".into());
    let shown = footer(&paint(&mut app, 160, 51));
    assert!(shown.contains("Retired binding requires migration."));
    assert!(!shown.contains("drag to resize"));
}

#[test]
fn every_visible_boundary_uses_real_painted_hits_and_restores_its_default() {
    use crate::board::shell::OpsTab;
    let _no_commands = crate::runner::forbid_commands();
    for (tab, width, id) in [
        (OpsTab::Overview, 148, Id::OverviewColumns),
        (OpsTab::Agents, 160, Id::AgentsColumns),
        (OpsTab::Agents, 100, Id::AgentsRows),
        (OpsTab::Digest, 160, Id::DigestColumns),
        (OpsTab::Digest, 100, Id::DigestRows),
        (OpsTab::Overview, 148, Id::OverviewUsage),
        (OpsTab::Overview, 103, Id::OverviewList),
        (OpsTab::Overview, 103, Id::OverviewLowerColumns),
        (OpsTab::Overview, 60, Id::OverviewLowerRows),
        (OpsTab::Overview, 148, Id::OverviewExchange),
        (OpsTab::Overview, 148, Id::OverviewTimers),
    ] {
        let mut app = populated_app(tab, crate::look::Look::default());
        paint(&mut app, width, 51);
        let get = |app: &App| {
            app.pane_resize
                .frame
                .borrow()
                .iter()
                .find(|b| b.id == id)
                .copied()
        };
        let b = get(&app).unwrap_or_else(|| panic!("{id:?} must expose a painted resize target"));
        let selected = app.workspace.selected.clone();
        let now = Instant::now();
        let delta = if b.before < b.max { 1i16 } else { -1 };
        let x = (b.hit.x as i32 + if id.columns() { i32::from(delta) } else { 0 }) as u16;
        let y = (b.hit.y as i32 + if id.columns() { 0 } else { i32::from(delta) }) as u16;
        assert_eq!(
            app.mouse(
                event(MouseEventKind::Down(MouseButton::Left), b.hit.x, b.hit.y),
                now
            ),
            Effect::None
        );
        assert_eq!(
            app.mouse(event(MouseEventKind::Drag(MouseButton::Left), x, y), now),
            Effect::None
        );
        paint(&mut app, width, 51);
        assert_eq!(
            app.mouse(event(MouseEventKind::Up(MouseButton::Left), x, y), now),
            Effect::None
        );
        paint(&mut app, width, 51);
        assert_eq!(
            get(&app).unwrap().before as i32,
            b.before as i32 + i32::from(delta),
            "{id:?}"
        );
        assert_eq!(app.workspace.selected, selected);
        assert!(app.menu.is_none() && app.input.is_none());
        let reset =
            crate::action::Action::parse(&format!("resize-pane {} reset", id.name())).unwrap();
        assert_eq!(app.perform(&reset), Effect::None);
        paint(&mut app, width, 51);
        assert_eq!(
            get(&app).unwrap().before,
            b.before,
            "{id:?} resets to the exact natural default"
        );
    }
}

#[test]
fn stored_stacked_lower_ratio_does_not_change_the_side_by_side_layout() {
    use crate::board::shell::OpsTab;
    let mut app = populated_app(OpsTab::Overview, crate::look::Look::default());
    let original = paint(&mut app, 103, 51);
    app.pane_resize
        .restore([(Id::OverviewLowerRows, 7000)].into());
    assert_eq!(paint(&mut app, 103, 51), original);
    paint(&mut app, 60, 51);
    assert!(
        app.pane_resize
            .frame
            .borrow()
            .iter()
            .any(|b| b.id == Id::OverviewLowerRows)
    );
}

#[test]
fn pointer_events_clear_a_drag_hover_on_terminals_without_motion_events() {
    use crate::board::shell::OpsTab;
    let mut app = app(OpsTab::Agents);
    paint(&mut app, 160, 51);
    let b = app.pane_resize.frame.borrow()[0];
    let now = Instant::now();
    app.mouse(
        event(MouseEventKind::Down(MouseButton::Left), b.hit.x, b.hit.y),
        now,
    );
    app.mouse(
        event(
            MouseEventKind::Drag(MouseButton::Left),
            b.hit.x + 2,
            b.hit.y,
        ),
        now,
    );
    paint(&mut app, 160, 51);
    app.mouse(
        event(MouseEventKind::Up(MouseButton::Left), b.hit.x + 2, b.hit.y),
        now,
    );
    assert_eq!(app.pane_resize.hint(), Some("↔"));
    app.mouse(event(MouseEventKind::ScrollDown, 0, 0), now);
    assert!(app.pane_resize.hint().is_none());
}

#[test]
fn empty_wide_exchange_can_resize_without_borrowing_a_recipient() {
    use crate::board::shell::OpsTab;
    let mut app = crate::board::home::tests::oracle::metered(crate::look::Look::default());
    app.shell.tab = OpsTab::Overview;
    app.overview_view = crate::board::overview::monitor::View::Squads;
    paint(&mut app, 148, 51);
    assert!(app.conversation_target().is_none());
    assert!(
        app.pane_resize
            .frame
            .borrow()
            .iter()
            .any(|b| b.id == Id::OverviewExchange)
    );
    let selected = app.workspace.selected.clone();
    app.perform(&crate::action::Action::parse("resize-pane overview-exchange down").unwrap());
    paint(&mut app, 148, 51);
    assert_eq!(app.workspace.selected, selected);
    assert!(app.input.is_none() && app.menu.is_none());
    assert!(app.conversation_target().is_none());
}

#[test]
fn minimum_column_widths_keep_required_table_fields_whole() {
    use crate::board::shell::OpsTab;
    let mut app = populated_app(OpsTab::Overview, crate::look::Look::default());
    app.pane_resize.restore([(Id::OverviewColumns, 0)].into());
    let buffer = paint(&mut app, 148, 51);
    let list_width = app
        .pane_resize
        .frame
        .borrow()
        .iter()
        .find(|b| b.id == Id::OverviewColumns)
        .unwrap()
        .before;
    let header = buffer
        .content
        .chunks(148)
        .map(|row| {
            row.iter()
                .take(usize::from(list_width))
                .map(|cell| cell.symbol())
                .collect::<String>()
        })
        .find(|line| line.contains("AGENT"))
        .expect("the actual painted table header");
    assert!(
        header.contains("TOK 1h") && header.contains("NOW"),
        "{header}"
    );
    assert_eq!(
        list_width,
        crate::board::conversation::layout::OVERVIEW_MIN_COLUMNS
    );
    app.pane_resize
        .restore([(Id::OverviewLowerColumns, 0)].into());
    paint(&mut app, 103, 51);
    assert_eq!(
        app.pane_resize
            .frame
            .borrow()
            .iter()
            .find(|b| b.id == Id::OverviewLowerColumns)
            .unwrap()
            .before,
        crate::board::timer_rows::MIN_CELLS
    );
    let row = crate::board::timer_rows::Row {
        id: "c1".into(),
        squad: "tmt-core".into(),
        schedule: "every 30m".into(),
        next: Some("in 9999m".into()),
        purpose: "purpose".into(),
        dim: false,
    };
    let shown = crate::board::timer_rows::line(
        &row,
        usize::from(crate::board::timer_rows::MIN_CELLS),
        crate::look::Look::default(),
    )
    .to_string();
    assert!(
        shown.contains("c1")
            && shown.contains("tmt-core")
            && shown.contains("every 30m")
            && shown.contains("in 9999m"),
        "{shown}"
    );
}

#[test]
fn geometry_invalidation_cancels_the_double_click_sequence() {
    let state = State::default();
    let b = boundary(Id::AgentsColumns);
    let now = Instant::now();
    state.save(b, 70);
    state.record(b, Rect::new(0, 0, 160, 100));
    state.mouse(
        event(MouseEventKind::Down(MouseButton::Left), b.hit.x, b.hit.y),
        now,
    );
    state.mouse(
        event(MouseEventKind::Up(MouseButton::Left), b.hit.x, b.hit.y),
        now,
    );
    state.invalidate();
    state.record(b, Rect::new(0, 0, 160, 100));
    assert_eq!(
        state.mouse(
            event(MouseEventKind::Down(MouseButton::Left), b.hit.x, b.hit.y),
            now + Duration::from_millis(100)
        ),
        Some(false)
    );
    assert_eq!(
        state.ratios()[&Id::AgentsColumns],
        7000,
        "a fresh paint cannot turn a stale first click into a reset"
    );
}

#[test]
fn keyboard_resize_stays_out_of_a_covering_actions_menu() {
    use crate::board::shell::OpsTab;
    let mut app = app(OpsTab::Agents);
    crate::board::app::tests::configure_workspace(
        &mut app,
        "[bind]\nr='resize-pane agents-columns right'\n",
    );
    paint(&mut app, 160, 51);
    assert_eq!(app.context_menu(), Effect::None);
    assert!(app.menu.as_ref().unwrap().entries.iter().all(|entry|!matches!(&entry.choice,crate::board::app::Choice::Action(action) if action.verb==crate::action::Verb::Ui(crate::action::OpsUiAction::ResizePane))));
}
