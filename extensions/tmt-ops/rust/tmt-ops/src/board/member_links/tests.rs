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
    fn named_inventory() -> Inventory {
        Inventory::from_listed(
            &[],
            &Ok(serde_json::json!({"identities":[
                {"id":"11111111-1111-4111-8111-111111111111", "name":"Night", "canonicalName":"night-2"},
                {"id":"22222222-2222-4222-8222-222222222222", "name":"Night long", "canonicalName":"night-2-extra"},
                {"id":"33333333-3333-4333-8333-333333333333", "name":"Ben", "canonicalName":"ben"},
                {"id":"44444444-4444-4444-8444-444444444444", "name":"Builder", "canonicalName":"builder"}
            ]})),
        )
    }
    #[test]
    fn canonical_bare_and_at_names_use_longest_complete_words_without_rewriting_source() {
        let text = "night-2. (@night-2-extra), @ben; ben builder @builder night-20 xnight-2 night-2x Night-2";
        let original = text.to_owned();
        let links = resolve(text, &named_inventory());
        assert_eq!(
            links
                .iter()
                .map(|link| &text[link.range.clone()])
                .collect::<Vec<_>>(),
            ["night-2", "@night-2-extra", "@ben", "@builder"]
        );
        assert_eq!(
            links
                .iter()
                .map(|link| link.name.as_str())
                .collect::<Vec<_>>(),
            ["night-2", "@night-2-extra", "@ben", "@builder"]
        );
        assert_eq!(text, original);
    }
    #[test]
    fn the_idle_alert_notice_commands_link_their_member_uuid_and_nothing_else() {
        let ada = "11111111-1111-4111-8111-111111111111";
        let stranger = "99999999-9999-4999-8999-999999999999";
        let text = format!(
            "Look: tmt focus {ada} · Talk: tmt talk {ada} \"<message>\" --detach\n\
             tmt focus {stranger} x{ada} {ada}0 @{ada}"
        );
        let links = resolve(&text, &inventory());
        assert_eq!(
            links
                .iter()
                .map(|link| (
                    &text[link.range.clone()],
                    link.identity.as_str(),
                    link.name.as_str()
                ))
                .collect::<Vec<_>>(),
            [(ada, ada, "Ada"), (ada, ada, "Ada")]
        );
    }
    #[test]
    fn unicode_word_neighbors_and_failed_inventory_never_create_partial_links() {
        let text = "é night-2 林night-2 night-2林; @ben_ @ben- @bené email@ben";
        let mut inventory = named_inventory();
        let links = resolve(text, &inventory);
        assert_eq!(
            links
                .iter()
                .map(|link| &text[link.range.clone()])
                .collect::<Vec<_>>(),
            ["night-2"]
        );
        inventory.error = Some("unavailable".into());
        assert!(resolve("night-2 @ben #tmt:night-2", &inventory).is_empty());
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
        assert_eq!(
            app.menu
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .map(|entry| entry.label.as_str())
                .collect::<Vec<_>>(),
            ["Message", "Jump to host", "Show in Agents"],
            "missing copy fields and an unobserved lead hide those actions"
        );
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
        let links: Vec<_> = mapped.mentions.iter().flatten().collect();
        // The ordinary URL opens a link popover, never a member menu.
        assert!(
            links
                .iter()
                .any(|link| link.target == Target::Url("https://example.com".into()))
        );
        let members: Vec<_> = links
            .iter()
            .filter(|link| matches!(link.target, Target::Member(_)))
            .collect();
        assert!(members.iter().all(|link| link.target
            == Target::Member(crate::board::app::workspace::tests::MEMBER.into())));
        assert_eq!(members.iter().map(|link| link.width).sum::<usize>(), 6);
    }
    #[test]
    fn painted_web_links_become_spans_that_share_an_id_per_link_and_skip_other_schemes() {
        let mut app = app();
        app.view.as_mut().unwrap().replies = vec![serde_json::json!({
            "recipientId":crate::board::app::workspace::tests::MEMBER,
            "requestId":"web-spans",
            "response":"See [docs](https://example.com/x), [docs](https://example.com/y) and [mail](mailto:a@example.com)."
        })];
        app.focus = 1;
        let mut terminal = Terminal::new(TestBackend::new(100, 36)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let spans = app.member_links.borrow().web_spans();
        // Two web links; the mail address stays a popover-only link.
        assert_eq!(spans.len(), 2, "{spans:?}");
        assert!(spans.iter().all(|span| span.area.width == 4));
        let open = |index: usize| String::from_utf8(spans[index].open().to_vec()).unwrap();
        assert!(
            open(0).contains(";https://example.com/x\x1b\\"),
            "{:?}",
            open(0)
        );
        assert!(
            open(1).contains(";https://example.com/y\x1b\\"),
            "{:?}",
            open(1)
        );
        assert_ne!(open(0).split(';').nth(1), open(1).split(';').nth(1));
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
            surface: Rect::default(),
            area: Rect::new(4, 10, 6, 1),
            target: Target::Member(MEMBER.into()),
            reader: true,
            occurrence: 0,
        });
        let choice = app
            .menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.label == "Show in Agents")
            .unwrap()
            .choice
            .clone();
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
        app.shell.tab = crate::board::shell::OpsTab::Agents;
        app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
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

mod named_surface_tests {
    use super::*;
    use crate::{
        action::{Action, Verb},
        board::app::workspace::tests::{MEMBER, ROOM, board},
        config::OpsKeyScope,
    };
    use ratatui::{Terminal, backend::TestBackend, style::Modifier};

    fn app() -> App {
        let mut app = board();
        app.references = Inventory::from_listed(
            &[],
            &Ok(serde_json::json!({"identities":[
                {"id":MEMBER, "name":"worker", "canonicalName":"night-2"}
            ]})),
        );
        app
    }
    fn open(app: &mut App) {
        app.member_links
            .borrow_mut()
            .begin_frame(Rect::new(0, 0, 100, 36));
        app.open_member_link(Hit {
            area: Rect::new(5, 5, 7, 1),
            surface: Rect::new(0, 5, 100, 1),
            target: Target::Member(MEMBER.into()),
            reader: false,
            occurrence: 0,
        });
    }
    #[test]
    fn member_menu_reuses_all_five_row_actions_and_revalidates_before_activation() {
        let mut app = app();
        let target = test_target();
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        snapshot.roster.squads[0].lead = Some(MEMBER.into());
        snapshot
            .keys
            .get_mut(&OpsKeyScope::Member)
            .unwrap()
            .insert("y".into(), Action::parse("copy {name}").unwrap());
        open(&mut app);
        assert_eq!(
            app.menu
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .map(|entry| entry.label.as_str())
                .collect::<Vec<_>>(),
            [
                "Message",
                "Jump to host",
                "Copy",
                "Note to lead",
                "Show in Agents"
            ]
        );
        let copy = app
            .menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.label == "Copy")
            .unwrap()
            .choice
            .clone();
        let message = app.menu.as_ref().unwrap().entries[0].choice.clone();
        let note = app
            .menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.label == "Note to lead")
            .unwrap()
            .choice
            .clone();
        app.menu = None;
        let selected = app.workspace.selected.clone();
        assert!(
            matches!(app.choose(copy.clone()), Effect::Act(Request::Copy {text, ..}) if text == "worker")
        );
        assert_eq!(app.workspace.selected, selected, "copy does not navigate");
        app.choose(message);
        assert!(app.input.is_some());
        assert_eq!(app.workspace.selected.as_ref(), Some(&target));
        assert_eq!(app.focus, 1);
        assert_eq!(app.shell.tab, crate::board::shell::OpsTab::Agents);
        app.input = None;
        app.choose(note);
        assert!(matches!(
            app.input.as_ref().unwrap().compose,
            crate::board::app::Compose::Annotate { .. }
        ));
        app.input = None;
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        snapshot
            .keys
            .get_mut(&OpsKeyScope::Member)
            .unwrap()
            .insert("y".into(), Action::parse("copy {id}").unwrap());
        assert_eq!(app.choose(copy), Effect::None);
        assert!(app.notice.as_ref().unwrap().contains("changed"));
        assert_eq!(app.workspace.selected.as_ref(), Some(&target));
        assert!(matches!(target, OpsTarget::Member {room_id:Some(ref room), ..} if room == ROOM));
    }
    #[test]
    fn plain_fields_cache_hits_and_never_link_a_name_created_by_clipping() {
        let app = app();
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let mut builds = None;
        for _ in 0..3 {
            terminal
                .draw(|frame| {
                    app.member_links.borrow_mut().begin_frame(frame.area());
                    let area = Rect::new(3, 2, 20, 1);
                    let row = crate::board::key_value::Row {
                        key: "task",
                        value: crate::board::key_value::Value::Observed("night-2"),
                    };
                    crate::board::key_value::paint(frame, &app, area, &row);
                })
                .unwrap();
            let current = app.member_links.borrow().builds;
            if let Some(previous) = builds {
                assert_eq!(
                    current, previous,
                    "unchanged fields reuse their projections"
                );
            } else {
                assert!(current > 0);
                builds = Some(current);
            }
        }
        let hit = app.member_links.borrow().hits[0].clone();
        assert_eq!(hit.area.width, 7);
        assert!(!hit.reader);
        for x in hit.area.x..hit.area.right() {
            let cell = &terminal.backend().buffer()[(x, hit.area.y)];
            assert!(cell.modifier.contains(Modifier::UNDERLINED));
            assert!(!cell.modifier.contains(Modifier::BOLD));
            assert_eq!(
                cell.fg,
                app.look()
                    .role(tmt_cli_style::Role::Text)
                    .fg
                    .unwrap_or(ratatui::style::Color::Reset)
            );
        }
        terminal
            .draw(|frame| {
                app.member_links.borrow_mut().begin_frame(frame.area());
                paint_plain_hits(
                    frame,
                    &app,
                    Rect::new(0, 0, 10, 1),
                    "night-20",
                    "night-2…",
                    false,
                );
            })
            .unwrap();
        assert!(app.member_links.borrow().hits.is_empty());
        assert!(
            app.member_link_binding(&test_target(), Verb::Copy)
                .is_none()
        );
    }
    #[test]
    fn plain_fields_use_the_resolved_occurrence_not_an_earlier_prefix() {
        let app = app();
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        for (source, displayed, expected) in [
            (
                "night-20 before night-2",
                "night-20 before night-2",
                Some(16),
            ),
            ("xnight-2 then night-2", "xnight-2 then night-2", Some(14)),
            ("night-20 before night-2", "night-2…", None),
        ] {
            terminal
                .draw(|frame| {
                    app.member_links.borrow_mut().begin_frame(frame.area());
                    let area = Rect::new(3, 2, 50, 1);
                    tmt_tui::components::strip::paint_left(
                        frame.buffer_mut(),
                        area,
                        ratatui::text::Line::raw(displayed),
                    );
                    paint_plain_hits(frame, &app, area, source, displayed, false);
                })
                .unwrap();
            let links = app.member_links.borrow();
            match expected {
                Some(column) => {
                    assert_eq!(links.hits.len(), 1, "{source}");
                    assert_eq!(links.hits[0].area.x, 3 + column, "{source}");
                }
                None => assert!(
                    links.hits.is_empty(),
                    "clipping must not borrow a later source occurrence"
                ),
            }
        }
    }
    fn paint_row(app: &App, source: &str) {
        let index = app
            .workspace_rows()
            .iter()
            .position(|row| Some(&row.target) == app.workspace.selected.as_ref())
            .unwrap();
        app.hits.borrow_mut().clear();
        app.hits.borrow_mut().push(crate::board::app::Hit {
            x: 2,
            y: 3,
            width: 54,
            target: crate::board::app::HitTarget::Row(index),
        });
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        terminal
            .draw(|frame| {
                app.member_links.borrow_mut().begin_frame(frame.area());
                let area = Rect::new(2, 3, 54, 1);
                tmt_tui::components::strip::paint_left(
                    frame.buffer_mut(),
                    area,
                    ratatui::text::Line::raw(source),
                );
                paint_plain_hits(frame, app, area, source, source, false);
            })
            .unwrap();
    }
    #[test]
    fn row_at_focus_preserves_fold_keys_and_authored_bindings_and_exits_on_movement() {
        let mut app = app();
        app.focus = 0;
        paint_row(&app, "night-2 then night-2");
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        assert!(app.row_names_hint());
        assert_eq!(
            app.member_link_key(key(KeyCode::Left)),
            None,
            "fold keeps its ordinary owner"
        );
        assert_eq!(
            app.member_link_key(key(KeyCode::Char('@'))),
            Some(Effect::None)
        );
        assert!(app.menu.is_none());
        assert_eq!(
            app.member_links
                .borrow()
                .focused
                .as_ref()
                .unwrap()
                .occurrence,
            0
        );
        app.member_link_key(key(KeyCode::Right));
        assert_eq!(
            app.member_links
                .borrow()
                .focused
                .as_ref()
                .unwrap()
                .occurrence,
            1
        );
        assert_eq!(app.member_link_hint(), Some("←→ name  ⏎ menu  Esc back"));
        app.member_link_key(key(KeyCode::Enter));
        assert!(app.menu.is_some());
        app.menu = None;
        app.member_link_key(key(KeyCode::Esc));
        assert!(app.member_links.borrow().focused.is_none());
        app.member_link_key(key(KeyCode::Char('@')));
        assert_eq!(app.member_link_key(key(KeyCode::Down)), None);
        assert!(app.member_links.borrow().focused.is_none());
        assert_eq!(
            app.member_link_key(key(KeyCode::Right)),
            None,
            "expand is restored"
        );
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
            .insert("@".into(), Action::parse("copy {name}").unwrap());
        assert!(!app.row_names_hint());
        assert_eq!(app.member_link_key(key(KeyCode::Char('@'))), None);
        assert!(
            matches!(app.key(key(KeyCode::Char('@'))), Effect::Act(Request::Copy {text, ..}) if text == "worker")
        );
    }
    #[test]
    fn single_row_link_opens_directly_and_optional_hint_never_evicts_core_keys() {
        let mut app = app();
        app.focus = 0;
        app.shell.tab = crate::board::shell::OpsTab::Agents;
        app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
        paint_row(&app, "ask night-2");
        let mut shown = 0;
        for width in 0..=160 {
            let baseline = {
                let held = std::mem::take(&mut app.member_links.borrow_mut().hits);
                let text = crate::board::view::footer::agents_hints(&app, width).unwrap();
                app.member_links.borrow_mut().hits = held;
                text
            };
            let actual = crate::board::view::footer::agents_hints(&app, width).unwrap();
            if actual.contains("@ names") {
                shown += 1;
                assert_eq!(actual.replacen("@ names  ", "", 1), baseline);
                assert!(actual.find("@ names").unwrap() < actual.find("? help").unwrap());
                assert!(actual.ends_with("q quit"));
            } else {
                assert_eq!(actual, baseline);
            }
        }
        assert!(shown > 0);
        app.member_link_key(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE));
        assert!(app.menu.is_some());
        assert!(app.member_links.borrow().focused.is_none());
    }
    fn rendered_scene(
        scene: &str,
        width: u16,
        look: crate::look::Look,
    ) -> (App, Terminal<TestBackend>) {
        let mut app = app();
        let target = test_target();
        app.view.as_mut().unwrap().look = look;
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        for held in snapshot.squad_looks.values_mut() {
            *held = look;
        }
        snapshot.rows.get_mut(&target).unwrap()["fields"] =
            serde_json::json!({"task":"Review night-2 then @night-2; ben builder stay plain"});
        app.overview_rows.borrow_mut().clear();
        if scene == "task" || scene == "notice" {
            app.shell.tab = crate::board::shell::OpsTab::Agents;
            app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
            // The agents list shows no free text; a squad's "now" line is the task surface.
            app.workspace.filter = crate::board::workspace::AgentsFilter::All;
            app.focus = 0;
        } else {
            app.shell.tab = crate::board::shell::OpsTab::Agents;
            app.focus = 1;
            app.view.as_mut().unwrap().replies = vec![serde_json::json!({
                "recipientId":MEMBER, "requestId":"name-link-source", "response":"Ask **@night-2** or night-2.\n> night-2 is still an exact quote.\nben builder stay plain."
            })];
        }
        if scene == "notice" {
            app.say("Saved view for night-2");
        }
        // A squad's "now" text needs a MODEL column wide enough to name a member whole.
        let width = if scene == "task" {
            width.max(300)
        } else {
            width
        };
        let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        if scene == "pane-output" {
            let key = app
                .pane_output_read()
                .expect("visible selected member can read pane output");
            app.apply_pane_output(
                key,
                crate::board::recent_responses::Observation::Observed(vec![
                    crate::board::recent_responses::Item {
                        provider: "codex".into(),
                        text: "Pane output mentions night-2 without request authority".into(),
                        ended_ms: crate::status::now_ms(),
                        truncated: false,
                    },
                ]),
            );
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
        }
        if scene == "menu" {
            let hit = app
                .member_links
                .borrow()
                .hits
                .iter()
                .find(|hit| hit.reader)
                .expect("exchange reference is visible")
                .clone();
            app.open_member_link(hit);
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
        }
        assert_eq!(app.look(), look);
        (app, terminal)
    }
    #[test]
    fn real_exchange_task_and_notice_surfaces_publish_only_visible_link_cells() {
        let _no_commands = crate::runner::forbid_commands();
        for scene in ["exchange", "task", "notice", "pane-output"] {
            let (app, terminal) = rendered_scene(scene, 160, crate::look::Look::default());
            let links = app.member_links.borrow();
            assert!(
                !links.hits.is_empty(),
                "{scene}: {:?}",
                terminal
                    .backend()
                    .buffer()
                    .content
                    .chunks(usize::from(terminal.backend().buffer().area.width))
                    .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                    .find(|row| row.contains("Review"))
            );
            for hit in &links.hits {
                assert_eq!(
                    hit.area.intersection(terminal.backend().buffer().area),
                    hit.area
                );
                let text = (hit.area.x..hit.area.right())
                    .map(|x| terminal.backend().buffer()[(x, hit.area.y)].symbol())
                    .collect::<String>();
                assert!(text.contains("night-2"), "{scene}: {text}");
            }
            if scene == "notice" {
                assert!(links.hits.iter().any(|hit| hit.area.y == 35));
            }
            if scene == "exchange" {
                assert_eq!(
                    app.view.as_ref().unwrap().replies[0]["response"],
                    "Ask **@night-2** or night-2.\n> night-2 is still an exact quote.\nben builder stay plain."
                );
            }
        }
    }
    #[test]
    #[ignore = "explicit member name-link renderer evidence export"]
    fn capture_member_names_packet() {
        let _no_commands = crate::runner::forbid_commands();
        let output = std::env::var("TMT_MEMBER_NAMES_PACKET").expect("task-owned output path");
        let mut frames = Vec::new();
        crate::status::with_now_ms(1_791_600_000_000, || {
            for scene in ["exchange", "task", "notice", "pane-output", "menu"] {
                for width in [160, 100, 60] {
                    for (base, depth) in [
                        ("tmt", tmt_cli_style::Depth::TrueColor),
                        ("tmt-light", tmt_cli_style::Depth::TrueColor),
                        ("tmt", tmt_cli_style::Depth::None),
                    ] {
                        let look = crate::look::Look {
                            theme: tmt_cli_style::Theme::new(
                                tmt_cli_style::Base::parse(base).unwrap(),
                            ),
                            depth,
                        };
                        let (app, terminal) = rendered_scene(scene, width, look);
                        let buffer = terminal.backend().buffer();
                        let lines: Vec<_> = buffer
                            .content
                            .chunks(usize::from(width))
                            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                            .collect();
                        let cells:Vec<_> = buffer.content.iter().map(|cell| serde_json::json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect();
                        let hits:Vec<_> = app.member_links.borrow().hits.iter().map(|hit|serde_json::json!({"x":hit.area.x,"y":hit.area.y,"width":hit.area.width,"height":hit.area.height,"identity":match &hit.target { Target::Member(id) | Target::Url(id) => id },"reader":hit.reader})).collect();
                        frames.push(serde_json::json!({"scene":scene,"width":width,"height":36,"base":base,"depth":format!("{depth:?}"),"lines":lines,"cells":cells,"memberHits":hits}));
                    }
                }
            }
        });
        assert_eq!(frames.len(), 45);
        std::fs::write(output, serde_json::to_vec(&frames).unwrap()).unwrap();
    }
    #[test]
    fn hover_uses_the_selected_row_background_and_leaves_links_normal_weight() {
        let mut app = app();
        let look = crate::look::Look {
            theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::Tmt),
            depth: tmt_cli_style::Depth::TrueColor,
        };
        app.view.as_mut().unwrap().look = look;
        for held in app
            .view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .squad_looks
            .values_mut()
        {
            *held = look;
        }
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let draw = |terminal: &mut Terminal<TestBackend>| {
            terminal
                .draw(|frame| {
                    app.member_links.borrow_mut().begin_frame(frame.area());
                    crate::board::key_value::paint(
                        frame,
                        &app,
                        Rect::new(2, 2, 40, 1),
                        &crate::board::key_value::Row {
                            key: "task",
                            value: crate::board::key_value::Value::Observed("night-2"),
                        },
                    );
                })
                .unwrap();
        };
        draw(&mut terminal);
        let hit = app.member_links.borrow().hits[0].clone();
        assert!(
            app.member_links
                .borrow_mut()
                .move_pointer((hit.area.x, hit.area.y).into(), true)
        );
        draw(&mut terminal);
        for x in hit.area.x..hit.area.right() {
            let cell = &terminal.backend().buffer()[(x, hit.area.y)];
            assert_eq!(cell.bg, look.selection().bg.unwrap());
            assert!(cell.modifier.contains(Modifier::UNDERLINED));
            assert!(!cell.modifier.contains(Modifier::BOLD));
        }
        assert!(
            app.member_links
                .borrow_mut()
                .move_pointer((0, 0).into(), true)
        );
        draw(&mut terminal);
        assert_ne!(
            terminal.backend().buffer()[(hit.area.x, hit.area.y)].bg,
            look.selection().bg.unwrap()
        );
    }
    #[test]
    fn anchored_member_menu_shortcut_letters_use_neutral_text_color() {
        for base in [tmt_cli_style::Base::Tmt, tmt_cli_style::Base::TmtLight] {
            let look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(base),
                depth: tmt_cli_style::Depth::TrueColor,
            };
            let (_app, terminal) = rendered_scene("menu", 100, look);
            let buffer = terminal.backend().buffer();
            let row = (0..buffer.area.height)
                .find_map(|y| {
                    let text = (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>();
                    let start = text.find("Copy")?;
                    Some((y, text[..start].chars().count() as u16 - 3))
                })
                .expect("Copy is available in this member menu");
            assert_eq!(buffer[(row.1, row.0)].symbol(), "y");
            assert_eq!(
                buffer[(row.1, row.0)].fg,
                look.role(tmt_cli_style::Role::Text).fg.unwrap()
            );
        }
    }
    #[test]
    fn rendered_exchange_names_use_text_color_even_after_reader_hit_paint() {
        for base in [tmt_cli_style::Base::Tmt, tmt_cli_style::Base::TmtLight] {
            let look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(base),
                depth: tmt_cli_style::Depth::TrueColor,
            };
            let (app, terminal) = rendered_scene("exchange", 160, look);
            let links = app.member_links.borrow();
            assert!(!links.hits.is_empty());
            for hit in &links.hits {
                for x in hit.area.x..hit.area.right() {
                    let cell = &terminal.backend().buffer()[(x, hit.area.y)];
                    assert_eq!(
                        cell.fg,
                        look.role(tmt_cli_style::Role::Text)
                            .fg
                            .unwrap_or(ratatui::style::Color::Reset)
                    );
                    assert!(cell.modifier.contains(Modifier::UNDERLINED));
                    assert!(!cell.modifier.contains(Modifier::BOLD));
                }
            }
        }
    }
    #[test]
    fn squad_now_mentions_do_not_take_a_same_named_row_title_as_their_geometry() {
        let (mut app, mut terminal) = rendered_scene("task", 160, crate::look::Look::default());
        let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
        snapshot.roster.members.get_mut(MEMBER).unwrap().name = "night-2".into();
        snapshot.rows.get_mut(&test_target()).unwrap()["name"] = serde_json::json!("night-2");
        snapshot.roster.squads[0].name = "night-2".into();
        app.overview_rows.borrow_mut().clear();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let row = app
            .hits
            .borrow()
            .iter()
            .find(|hit| hit.target == crate::board::app::HitTarget::Row(1))
            .cloned()
            .unwrap();
        let links = app.member_links.borrow();
        let task_links: Vec<_> = links
            .hits
            .iter()
            .filter(|hit| !hit.reader && hit.area.y == row.y)
            .collect();
        assert_eq!(task_links.len(), 2);
        assert!(
            task_links
                .iter()
                .all(|hit| hit.surface.x > row.x + 13 && hit.area.x >= hit.surface.x),
            "task links must stay in the Now column, not the same named NAME column: {task_links:?}"
        );
    }
}
