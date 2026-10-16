//! Parser, current-frame geometry, effect and source-authority regressions.
use super::*;
use crate::board::app::Request;
use ratatui::crossterm::event::KeyModifiers;

fn test_target() -> OpsTarget {
    OpsTarget::Member {
        room_id: Some(crate::board::app::workspace::tests::ROOM.into()),
        identity_id: crate::board::app::workspace::tests::MEMBER.into(),
    }
}

mod parser_tests {
    use super::*;
    fn inventory() -> Inventory {
        Inventory::from_listed(
            &[],
            &Ok(serde_json::json!({"identities":[
                {"id":"11111111-1111-4111-8111-111111111111", "name":"Ada", "canonicalName":"ada"},
                {"id":"22222222-2222-4222-8222-222222222222", "name":"林", "canonicalName":"林"}
            ]})),
        )
    }
    #[test]
    fn exact_canonical_and_uuid_references_preserve_raw_unicode_ranges() {
        let raw = "請#tmt:ada，問#tmt:林。 (#tmt:11111111-1111-4111-8111-111111111111)!";
        let links = resolve(raw, &inventory());
        assert_eq!(links.len(), 3);
        assert_eq!(
            links.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            ["Ada", "林", "Ada"]
        );
        assert_eq!(&raw[links[1].range.clone()], "#tmt:林");
        assert_eq!(links[0].identity, links[2].identity);
    }
    #[test]
    fn unknown_prefixes_case_aliases_and_failed_inventory_stay_literal() {
        let mut inventory = inventory();
        assert!(resolve("#tmt:adab #tmt:ada-other #tmt:Ada #tmt:unknown", &inventory).is_empty());
        assert_eq!(resolve("#tmt:ada.#tmt:ada", &inventory).len(), 2);
        inventory.error = Some("unavailable".into());
        assert!(resolve("#tmt:ada", &inventory).is_empty());
    }
}

mod interaction_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    fn app() -> App {
        let mut app = crate::board::app::workspace::tests::board();
        app.references = Inventory::from_listed(
            &[],
            &Ok(
                serde_json::json!({"identities":[{"id": crate::board::app::workspace::tests::MEMBER,"name":"worker","canonicalName":"worker"}]}),
            ),
        );
        app
    }
    fn paint(app: &App, width: u16, height: u16, y: u16) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                app.member_links.borrow_mut().begin_frame(frame.area());
                assert!(paint_preview(
                    frame,
                    app,
                    Rect::new(2, y, width - 4, 1),
                    "Ask #tmt:worker, please.",
                    tmt_cli_style::Role::Link
                ));
            })
            .unwrap();
    }
    #[test]
    fn anchored_menu_flips_clamps_and_never_covers_the_mention() {
        for width in [60, 100, 160] {
            let screen = Rect::new(0, 0, width, 24);
            for y in [1, 10, 22] {
                let anchor = Rect::new(width - 8, y, 6, 1);
                let menu = menu_area(screen, anchor, 6).unwrap();
                assert_eq!(menu.intersection(screen), menu);
                assert!(menu.intersection(anchor).is_empty());
                assert_eq!(menu.height, 6);
                if y == 22 {
                    assert_eq!(menu.bottom(), anchor.y);
                } else {
                    assert_eq!(menu.y, anchor.bottom());
                }
            }
        }
        assert!(menu_area(Rect::new(0, 0, 60, 4), Rect::new(0, 1, 5, 1), 6).is_none());
    }
    #[test]
    fn preview_hits_open_one_menu_and_outside_click_dismisses_without_action() {
        let mut app = app();
        paint(&app, 60, 24, 3);
        let hit = app.member_links.borrow().hits[0].clone();
        assert_eq!(hit.area, Rect::new(6, 3, 6, 1));
        assert_eq!(
            app.member_link_mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 6,
                row: 3,
                modifiers: KeyModifiers::NONE
            }),
            Some(Effect::None)
        );
        assert!(app.menu.is_some());
        assert_eq!(app.menu.as_ref().unwrap().entries.len(), 2);
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now(),
        );
        assert!(app.menu.is_none());
        app.invalidate_frame_geometry();
        assert!(
            app.member_link_mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 6,
                row: 3,
                modifiers: KeyModifiers::NONE
            })
            .is_none()
        );
    }
    #[test]
    fn keyboard_uses_only_painted_reader_mentions_and_enter_opens_actions() {
        let mut app = app();
        app.focus = 1;
        paint(&app, 100, 24, 3);
        assert!(
            app.member_link_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
                .is_none(),
            "composer previews do not become conversation read stops"
        );
        app.member_links.borrow_mut().hits[0].reader = true;
        assert_eq!(
            app.member_link_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
            Some(Effect::None)
        );
        assert_eq!(
            app.member_link_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Effect::None)
        );
        assert!(app.menu.is_some());
    }
    #[test]
    fn menu_jump_rechecks_binding_and_uuid_then_show_agents_keeps_draft() {
        use crate::board::app::workspace::tests::MEMBER;
        let mut app = app();
        assert_eq!(
            app.member_link_action(&test_target(), true),
            Effect::Act(Request::Jump(MEMBER.into()))
        );
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
            .unwrap()["presence"] = serde_json::json!("offline");
        assert_eq!(app.member_link_action(&test_target(), true), Effect::None);
        assert!(app.notice.as_ref().unwrap().contains("offline"));
        app.perform(&crate::action::Action::parse("talk").unwrap());
        app.input.as_mut().unwrap().text = "keep #tmt:worker raw".into();
        app.member_link_action(&test_target(), false);
        assert_eq!(app.shell.tab, crate::board::shell::OpsTab::Agents);
        app.perform(&crate::action::Action::parse("talk").unwrap());
        assert_eq!(
            app.input.as_ref().unwrap().text.as_str(),
            "keep #tmt:worker raw"
        );
        app.references.agents.clear();
        assert_eq!(app.member_link_action(&test_target(), true), Effect::None);
        assert!(app.notice.as_ref().unwrap().contains("unavailable"));
    }
    #[test]
    fn styled_wrapping_retains_uuid_tags_but_never_turns_ordinary_urls_into_member_actions() {
        let app = app();
        let source = "[worker](https://example.com) **#tmt:worker**, #tmt:missing";
        let mapped = crate::board::markdown::render_with_members(
            source,
            12,
            app.look(),
            Some(&app.references),
        );
        let text: String = mapped
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .map(|span| span.content.as_ref())
            .collect();
        assert!(!text.contains("#tmt:worker"));
        assert!(text.contains("#tmt:missing"));
        assert!(
            mapped
                .mentions
                .iter()
                .flatten()
                .all(|link| link.identity == crate::board::app::workspace::tests::MEMBER)
        );
        assert_eq!(
            mapped
                .mentions
                .iter()
                .flatten()
                .map(|link| link.width)
                .sum::<usize>(),
            6
        );
    }
    fn painted_reader() -> App {
        let mut app = app();
        app.view.as_mut().unwrap().replies = vec![serde_json::json!({
            "recipientId":crate::board::app::workspace::tests::MEMBER,
            "requestId":"mention-navigation", "response":"Read #tmt:worker."
        })];
        app.focus = 1;
        let mut terminal = Terminal::new(TestBackend::new(100, 36)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        assert!(app.member_links.borrow().hits.iter().any(|hit| hit.reader));
        app
    }
    #[test]
    fn tab_and_backtab_keep_switching_panes_with_a_visible_focused_mention() {
        let mut app = painted_reader();
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert!(app.member_links.borrow().focused.is_some());
        for code in [
            KeyCode::Tab,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::BackTab,
        ] {
            let before = app.focus;
            assert_eq!(
                app.key(KeyEvent::new(code, KeyModifiers::NONE)),
                Effect::None
            );
            assert_ne!(app.focus, before, "{code:?} keeps its pane action");
            assert!(app.menu.is_none());
            let mut terminal = Terminal::new(TestBackend::new(100, 36)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            assert!(app.member_links.borrow().hits.iter().any(|hit| hit.reader));
        }
    }
    #[test]
    fn authored_reader_bindings_win_over_mention_navigation_and_activation() {
        use crate::board::app::workspace::tests::MEMBER;
        for (event, code) in [
            ("left", KeyCode::Left),
            ("right", KeyCode::Right),
            ("enter", KeyCode::Enter),
            ("tab", KeyCode::Tab),
            ("backtab", KeyCode::BackTab),
        ] {
            let mut app = painted_reader();
            app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
            assert!(app.member_links.borrow().focused.is_some());
            let path = std::env::temp_dir().join(format!(
                "ops-member-link-binding-{}-{event}.toml",
                std::process::id()
            ));
            std::fs::write(&path, format!("[bind]\n{event} = 'run recorder {{id}}'\n")).unwrap();
            let config = crate::config::Config::read(path.clone()).unwrap();
            app.view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .configure(&config)
                .unwrap();
            std::fs::remove_file(path).unwrap();
            assert_eq!(
                app.key(KeyEvent::new(code, KeyModifiers::NONE)),
                Effect::Act(Request::Run(vec!["recorder".into(), MEMBER.into()])),
                "authored {event} must reach the ordinary binding dispatcher"
            );
            assert!(app.menu.is_none());
        }
    }
}

mod additional_tests {
    use super::*;
    use crate::board::app::workspace::tests::{MEMBER, board};
    use ratatui::{Terminal, backend::TestBackend};
    #[test]
    fn wrapped_reference_is_one_keyboard_stop_and_repeated_occurrences_are_distinct() {
        let mut app = board();
        app.focus = 1;
        app.references = Inventory::from_listed(
            &[],
            &Ok(
                serde_json::json!({"identities":[{"id":MEMBER,"name":"Long Worker Name","canonicalName":"worker"}]}),
            ),
        );
        let mapped = crate::board::markdown::render_with_members(
            "#tmt:worker / #tmt:worker",
            8,
            app.look(),
            Some(&app.references),
        );
        assert!(mapped.mentions.iter().flatten().count() > 2);
        let mut terminal = Terminal::new(TestBackend::new(8, 24)).unwrap();
        terminal
            .draw(|frame| {
                app.member_links.borrow_mut().begin_frame(frame.area());
                paint_hits(
                    frame,
                    &app,
                    frame.area(),
                    0,
                    24,
                    &mapped.mentions,
                    Paint {
                        reader: true,
                        role: tmt_cli_style::Role::Link,
                    },
                );
            })
            .unwrap();
        app.member_link_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(
            app.member_links
                .borrow()
                .focused
                .as_ref()
                .unwrap()
                .occurrence,
            0
        );
        app.member_link_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(
            app.member_links
                .borrow()
                .focused
                .as_ref()
                .unwrap()
                .occurrence,
            1
        );
        app.member_link_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(
            app.member_links
                .borrow()
                .focused
                .as_ref()
                .unwrap()
                .occurrence,
            0
        );
    }
    #[test]
    fn configured_host_argv_is_resolved_for_the_mentioned_uuid_without_changing_selection() {
        let mut app = board();
        app.references = Inventory::from_listed(
            &[],
            &Ok(
                serde_json::json!({"identities":[{"id":MEMBER,"name":"worker","canonicalName":"worker"}]}),
            ),
        );
        let target = app.workspace.selected.clone().unwrap();
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        snapshot.rows.get_mut(&target).unwrap()["pane"]["location"]["host"] =
            serde_json::json!("herdr");
        snapshot
            .keys
            .get_mut(&crate::config::OpsKeyScope::Member)
            .unwrap()
            .insert(
                "j".into(),
                crate::action::Action::parse("run recorder {id}").unwrap(),
            );
        app.invoker_tmux = false;
        let opening = OpsTarget::Squad(crate::board::app::workspace::tests::ROOM.into());
        app.workspace.selected = Some(opening.clone());
        assert_eq!(
            app.member_link_action(&test_target(), true),
            Effect::Act(Request::Run(vec!["recorder".into(), MEMBER.into()]))
        );
        assert_eq!(app.workspace.selected.as_ref(), Some(&opening));
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        snapshot.rows.get_mut(&target).unwrap()["pane"]["location"]["host"] =
            serde_json::json!("tmux");
        let lead = "33333333-3333-4333-8333-333333333333";
        snapshot.roster.squads[0].lead = Some(lead.into());
        snapshot
            .keys
            .get_mut(&crate::config::OpsKeyScope::Member)
            .unwrap()
            .insert(
                "j".into(),
                crate::action::Action::parse("jump lead").unwrap(),
            );
        app.invoker_tmux = true;
        assert_eq!(
            app.member_link_action(&test_target(), true),
            Effect::None,
            "retired implicit lead targeting is refused by the shared action admission"
        );
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .contains("implicit lead targeting is retired")
        );
        assert_eq!(app.workspace.selected.as_ref(), Some(&opening));
    }
}

mod visibility_tests {
    use super::*;
    #[test]
    fn hidden_selection_keeps_ordinary_global_keys_while_explicit_mention_uses_its_host_context() {
        use crate::board::app::workspace::tests::{MEMBER, ROOM, board};
        use crate::config::OpsKeyScope;
        let mut app = board();
        app.references = Inventory::from_listed(
            &[],
            &Ok(
                serde_json::json!({"identities":[{"id":MEMBER,"name":"worker","canonicalName":"worker"}]}),
            ),
        );
        let target = app.workspace.selected.clone().unwrap();
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        snapshot.preferences.hidden.insert(ROOM.into());
        let action = crate::action::Action::parse("run recorder {id}").unwrap();
        snapshot.hosts.insert(
            "tmux".into(),
            [(OpsKeyScope::Member, [("j".into(), action.clone())].into())].into(),
        );
        app.workspace_reconcile();
        assert!(!app.workspace_selection_visible());
        assert_eq!(
            app.bindings()["j"].verb,
            crate::action::Verb::Jump,
            "hidden row supplies no implicit host override"
        );
        assert_eq!(
            app.member_link_jump(&target).unwrap(),
            action,
            "explicit UUID reference uses its own live host context"
        );
        assert_eq!(
            app.member_link_action(&test_target(), true),
            Effect::Act(Request::Run(vec!["recorder".into(), MEMBER.into()]))
        );
        assert!(
            !app.workspace.visibility.reveal_hidden,
            "jumping does not reveal or navigate hidden rows"
        );
    }
}

mod frozen_membership_tests {
    use super::*;
    use crate::board::app::workspace::tests::{MEMBER, board};
    fn app() -> App {
        let mut app = board();
        app.references = Inventory::from_listed(
            &[],
            &Ok(
                serde_json::json!({"identities":[{"id":MEMBER,"name":"worker","canonicalName":"worker"}]}),
            ),
        );
        app
    }
    #[test]
    fn menu_refuses_the_frozen_membership_even_when_the_identity_now_has_a_different_context() {
        let mut app = app();
        let target = test_target();
        app.member_links
            .borrow_mut()
            .begin_frame(Rect::new(0, 0, 100, 36));
        app.open_member_link(Hit {
            area: Rect::new(4, 10, 6, 1),
            identity: MEMBER.into(),
            reader: true,
            occurrence: 0,
        });
        let choice = app.menu.as_ref().unwrap().entries[1].choice.clone();
        assert!(
            matches!(&choice, Choice::MemberLink { target: retained, .. } if retained == &target)
        );
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        let row = snapshot.rows.remove(&target).unwrap();
        snapshot.roster.squads[0].members.clear();
        snapshot.roster.squads[0].member_states.clear();
        let pool = OpsTarget::Member {
            room_id: None,
            identity_id: MEMBER.into(),
        };
        snapshot.rows.insert(pool.clone(), row);
        assert_eq!(
            app.member_link_target(MEMBER),
            Some(pool),
            "the identity still resolves; only its frozen membership changed"
        );
        assert_eq!(app.choose(choice), Effect::None);
        assert!(app.notice.as_ref().unwrap().contains("membership changed"));
        assert_eq!(app.workspace.selected, Some(target));
    }
    #[test]
    fn flat_overview_identity_resolves_an_acquired_membership_before_showing_agents() {
        let mut app = app();
        app.shell.tab = crate::board::shell::OpsTab::Overview;
        app.workspace.selected = Some(OpsTarget::Member {
            room_id: None,
            identity_id: MEMBER.into(),
        });
        app.workspace_reconcile();
        let target = app.member_link_target(MEMBER).unwrap();
        assert_eq!(
            target,
            test_target(),
            "a flat monitor occurrence is not mistaken for Common pool membership"
        );
        app.member_link_action(&target, false);
        assert_eq!(app.shell.tab, crate::board::shell::OpsTab::Agents);
        assert_eq!(app.workspace.selected, Some(test_target()));
    }
}
