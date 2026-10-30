use super::*;
use crate::board::{
    app::workspace::tests::{MEMBER, ROOM, board},
    recent_responses::{Page as Answers, timeline},
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
};
use serde_json::{Value, json};
use std::sync::Arc;

const ACTOR: &str = "33333333-3333-4333-8333-333333333333";
const OTHER: &str = "66666666-6666-4666-8666-666666666666";

/// A second member `core-ops` in the same squad as `worker`.
fn app_with_two_members() -> App {
    let mut app = board();
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let mut member = snapshot.roster.members[MEMBER].clone();
    member.id = OTHER.into();
    member.name = "core-ops".into();
    snapshot.roster.members.insert(OTHER.into(), member);
    snapshot.roster.squads[0].members.push(OTHER.into());
    app.overview_rows.borrow_mut().clear();
    app
}

fn answers(items: &[(&str, &str, u64, &str)]) -> Value {
    let items: Vec<_> = items
        .iter()
        .map(|(member, request, at, kind)| {
            json!({"requestId":request,"recipientId":member,"sender":{"identityId":ACTOR},
                "kind":"request","answeredAtMs":at,"answer":kind,"revision":1,"acknowledged":false})
        })
        .collect();
    json!({"items":items,"nextBefore":null})
}

fn install(app: &mut App, items: &[(&str, &str, u64, &str)]) {
    app.home_leads
        .test_pages(ACTOR, json!({"items":[],"nextBefore":null}), answers(items));
    app.overview_rows.borrow_mut().clear();
}

/// The member's loaded answers: one reply per `(request, text)`.
fn replies(member: &str, items: &[(&str, &str)]) -> Observation {
    let entries: Vec<_> = items
        .iter()
        .map(|(request, text)| {
            json!({"entryId":format!("entry-{request}"),"inputBatchId":"batch","kind":"reply",
                "requestId":request,"availability":"retained","text":text,"truncated":false,
                "originalBytes":text.len(),
                "acknowledgements":[{"identityId":ACTOR,"requestId":request,"revision":1}],
                "acknowledgementsTruncated":false})
        })
        .collect();
    let document = json!({"identityId":member,"entries":entries,"responses":[],
        "coverage":{"olderDropped":false},"capture":{"enabled":true}});
    Observation::Observed(Arc::new(Answers {
        timeline: Some(timeline::parse(member, &document).unwrap()),
        ..Default::default()
    }))
}

/// A submitted reply and captured final whose metadata survives without their text.
fn unavailable_answers(member: &str, reply: &str, turn: &str) -> Observation {
    let document = json!({"identityId":member,"entries":[
        {"entryId":"reply-entry","kind":"reply","requestId":reply,
         "availability":"unavailable","acknowledgements":[],"acknowledgementsTruncated":false},
        {"entryId":"turn-entry","kind":"final","responseId":"rsp_unavailable",
         "availability":"unavailable","acknowledgements":[
             {"identityId":ACTOR,"requestId":turn,"revision":1}],
         "acknowledgementsTruncated":false}
    ],"responses":[],"coverage":{"olderDropped":false},"capture":{"enabled":true}});
    Observation::Observed(Arc::new(Answers {
        timeline: Some(timeline::parse(member, &document).unwrap()),
        ..Default::default()
    }))
}

fn draw(app: &App, height: u16) -> String {
    draw_size(app, 160, height)
}

fn draw_size(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn arrive(app: &mut App, member: &str, observation: Observation) {
    let session = app.unread_overlay.as_ref().unwrap().session;
    let (_, read) = app.unread_overlay_read().unwrap_or_else(|| {
        (
            session,
            recent_responses::AnswerRead {
                key: (Some(ACTOR.into()), member.into()),
                before: None,
                held: None,
                rebuild: false,
            },
        )
    });
    let read = recent_responses::AnswerRead {
        key: (read.key.0, member.into()),
        ..read
    };
    app.unread_overlay_arrived(session, &read, observation);
}

/// The overlay scenes the home oracle records.
pub(in crate::board) const SCENES: &[&str] = &[
    "unread-overlay-all",
    "unread-overlay-agent",
    "unread-overlay-loading",
    "unread-overlay-failed",
    "unread-overlay-unavailable",
    "unread-overlay-older",
    "unread-overlay-detail",
    "unread-overlay-read",
    "unread-overlay-confirm",
    "unread-overlay-empty",
    "unread-overlay-hover",
];

/// An opened overlay: two agents' unread answers, the way `scene` names.
pub(in crate::board) fn capture_case(scene: &str, look: crate::look::Look) -> App {
    let now = crate::status::now_ms();
    let mut app = app_with_two_members();
    // Keep approved clock cells independent of the host's system time zone.
    app.local_zone = Some(jiff::tz::TimeZone::get("Asia/Tokyo").unwrap());
    let view = app.view.as_mut().unwrap();
    view.look = look;
    // A saved squad look would win over the scene's, and it follows the environment.
    view.home.as_mut().unwrap().workspace.squad_looks.clear();
    install(
        &mut app,
        &[
            (MEMBER, "w-old", now - 2_000_000, "final"),
            (
                MEMBER,
                "w-new",
                now - 600_000,
                if scene == "unread-overlay-unavailable" {
                    "turn"
                } else {
                    "final"
                },
            ),
            (OTHER, "c-one", now - 900_000, "final"),
        ],
    );
    let scope = if scene == "unread-overlay-agent" {
        UnreadScope::Member(MEMBER.into())
    } else {
        UnreadScope::All
    };
    app.open_unread(&scope);
    let worker = replies(
        MEMBER,
        &[
            ("w-old", "Rebased the branch.\n\nThe checks are green."),
            ("w-new", "Opened the pull request."),
        ],
    );
    let core = replies(OTHER, &[("c-one", "The migration is in.")]);
    match scene {
        "unread-overlay-unavailable" => {
            arrive(
                &mut app,
                MEMBER,
                unavailable_answers(MEMBER, "w-old", "w-new"),
            );
            arrive(&mut app, OTHER, replies(OTHER, &[]));
        }
        "unread-overlay-older" => {
            // Settle the shown page, then deliver the real continuation event shape.
            let items = app
                .unread_overlay
                .as_ref()
                .unwrap()
                .items
                .values()
                .cloned()
                .map(|answer| (answer, crate::board::unread::publish::Outcome::Acknowledged))
                .collect();
            app.finished_unread(Ok(crate::board::unread::publish::Completed {
                actor: ACTOR.into(),
                items,
            }));
            let cursor = json!({"answeredAtMs":now-2_000_000,"requestId":"w-old"});
            app.unread_overlay.as_mut().unwrap().next_before = Some(cursor);
            let (session, read) = app.unread_overlay_page().unwrap();
            let older = Page::read(
                ACTOR,
                &answers(&[(MEMBER, "older-answer", now - 3_000_000, "final")]),
            )
            .unwrap();
            app.unread_overlay_page_arrived(session, &read, Ok(older));
            arrive(&mut app, MEMBER, replies(MEMBER, &[]));
            arrive(&mut app, OTHER, core);
        }
        "unread-overlay-loading" => {}
        "unread-overlay-failed" => {
            arrive(&mut app, MEMBER, worker);
            arrive(&mut app, OTHER, Observation::Unavailable);
        }
        "unread-overlay-agent" => arrive(&mut app, MEMBER, worker),
        _ => {
            arrive(&mut app, MEMBER, worker);
            arrive(&mut app, OTHER, core);
        }
    }
    match scene {
        "unread-overlay-detail" => app.unread_overlay.as_mut().unwrap().detail_open = true,
        "unread-overlay-read" => {
            let items = app
                .unread_overlay
                .as_ref()
                .unwrap()
                .items
                .values()
                .cloned()
                .map(|answer| (answer, crate::board::unread::publish::Outcome::Acknowledged))
                .collect();
            app.finished_unread(Ok(crate::board::unread::publish::Completed {
                actor: ACTOR.into(),
                items,
            }));
        }
        "unread-overlay-confirm" => {
            press(&mut app, KeyCode::Char('a'));
        }
        "unread-overlay-empty" => {
            install(&mut app, &[]);
            app.open_unread(&UnreadScope::All);
        }
        "unread-overlay-hover" => app.unread_overlay.as_ref().unwrap().hover.set(Some(1)),
        _ => {}
    }
    app
}

#[test]
fn the_overlay_lists_unread_answers_by_agent_oldest_first_with_a_count_per_group() {
    crate::status::with_now_ms(10_000_000, || {
        let mut app = app_with_two_members();
        install(
            &mut app,
            &[
                (MEMBER, "w-new", 9_000_000, "final"),
                (MEMBER, "w-old", 8_000_000, "final"),
                (OTHER, "c-one", 9_500_000, "final"),
            ],
        );
        assert_eq!(app.open_unread(&UnreadScope::All), Effect::None);
        arrive(
            &mut app,
            MEMBER,
            replies(
                MEMBER,
                &[
                    ("w-old", "Older worker answer"),
                    ("w-new", "Newer worker answer"),
                ],
            ),
        );
        arrive(&mut app, OTHER, replies(OTHER, &[("c-one", "Core answer")]));
        let screen = draw(&app, 40);
        assert!(
            screen.contains("Unread · 3 answers from 2 agents"),
            "{screen}"
        );
        assert!(screen.contains("── worker · 2"), "{screen}");
        assert!(screen.contains("── core-ops · 1"), "{screen}");
        let at = |text: &str| {
            screen
                .find(text)
                .unwrap_or_else(|| panic!("{text}\n{screen}"))
        };
        assert!(
            at("Older worker answer") < at("Newer worker answer"),
            "{screen}"
        );
        assert!(at("── core-ops · 1") < at("── worker · 2"), "{screen}");
        assert!(screen.contains("r mark read"), "{screen}");
    });
}

#[test]
fn a_late_answer_is_appended_below_and_never_moves_what_is_shown() {
    crate::status::with_now_ms(10_000_000, || {
        let mut app = app_with_two_members();
        install(
            &mut app,
            &[
                (MEMBER, "w", 8_000_000, "final"),
                (OTHER, "c", 9_000_000, "final"),
            ],
        );
        app.open_unread(&UnreadScope::All);
        arrive(&mut app, MEMBER, replies(MEMBER, &[("w", "Worker answer")]));
        arrive(&mut app, OTHER, replies(OTHER, &[("c", "Core answer")]));
        let before = draw(&app, 40);
        let row = |screen: &str, text: &str| {
            screen
                .lines()
                .position(|line| line.contains(text))
                .unwrap_or_else(|| panic!("{text}\n{screen}"))
        };
        let (core, worker) = (row(&before, "Core answer"), row(&before, "Worker answer"));
        assert!(core < worker, "the list's order at open: {before}");
        // A newer answer of the first group arrives while the overlay is open.
        install(
            &mut app,
            &[
                (MEMBER, "w", 8_000_000, "final"),
                (OTHER, "c", 9_000_000, "final"),
                (OTHER, "c2", 9_500_000, "final"),
            ],
        );
        app.unread_drawn();
        let during = draw(&app, 40);
        assert_eq!(row(&during, "Core answer"), core, "{during}");
        assert_eq!(
            app.unread_overlay_read().map(|(_, read)| read.key.1),
            Some(OTHER.to_owned()),
            "the member with a late answer is read again"
        );
        arrive(
            &mut app,
            OTHER,
            replies(OTHER, &[("c", "Core answer"), ("c2", "Late core answer")]),
        );
        let after = draw(&app, 40);
        assert!(
            after.contains("Unread · 3 answers from 2 agents"),
            "{after}"
        );
        assert_eq!(row(&after, "Core answer"), core, "{after}");
        assert_eq!(row(&after, "Worker answer"), worker, "{after}");
        let late = row(&after, "Late core answer");
        assert!(late > worker, "appended below the shown blocks: {after}");
        let rules: Vec<_> = after
            .lines()
            .filter(|l| l.contains("── core-ops"))
            .collect();
        assert_eq!(rules.len(), 2, "the group rule repeats: {after}");
    });
}

#[test]
fn ages_in_the_headers_follow_the_clock_while_the_overlay_stays_open() {
    let mut app = app_with_two_members();
    let drawn = |app: &App, now: u64| crate::status::with_now_ms(now, || draw(app, 40));
    crate::status::with_now_ms(10_000_000, || {
        install(&mut app, &[(MEMBER, "w", 8_000_000, "final")]);
        app.open_unread(&UnreadScope::Member(MEMBER.into()));
        arrive(&mut app, MEMBER, replies(MEMBER, &[("w", "An answer")]));
    });
    let first = drawn(&app, 10_000_000);
    assert!(
        first
            .lines()
            .any(|line| line.contains("worker → you") && line.contains("33m ago")),
        "{first}"
    );
    let later = drawn(&app, 10_000_000 + 20 * 60_000);
    assert!(
        later
            .lines()
            .any(|line| line.contains("worker → you") && line.contains("53m ago")),
        "{later}"
    );
}

#[test]
fn only_answers_painted_in_the_viewport_are_handed_to_the_writer() {
    crate::status::with_now_ms(10_000_000, || {
        let mut app = app_with_two_members();
        install(
            &mut app,
            &[
                (MEMBER, "first", 8_000_000, "final"),
                (MEMBER, "second", 9_000_000, "final"),
            ],
        );
        app.open_unread(&UnreadScope::Member(MEMBER.into()));
        let long = (0..120)
            .map(|n| format!("paragraph {n}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        arrive(
            &mut app,
            MEMBER,
            replies(MEMBER, &[("first", &long), ("second", "Second answer")]),
        );
        draw(&app, 30);
        app.unread_drawn();
        let seen = |app: &App| {
            app.unread
                .displayed
                .iter()
                .map(|body| body.request.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(seen(&app), ["first"], "the second answer is below the fold");
        for _ in 0..40 {
            press(&mut app, KeyCode::PageDown);
        }
        let screen = draw(&app, 30);
        app.unread_drawn();
        assert!(screen.contains("Second answer"), "{screen}");
        assert!(seen(&app).contains(&"second".to_owned()));
        assert_eq!(
            app.unread_count(MEMBER).total,
            2,
            "nothing acknowledged before Core says so"
        );
    });
}

#[test]
fn a_failed_read_says_so_and_is_asked_again_only_after_scrolling_away_and_back() {
    crate::status::with_now_ms(10_000_000, || {
        let mut app = app_with_two_members();
        install(
            &mut app,
            &[
                (OTHER, "w", 8_000_000, "final"),
                (MEMBER, "c", 9_000_000, "final"),
            ],
        );
        app.open_unread(&UnreadScope::All);
        arrive(&mut app, OTHER, Observation::Unavailable);
        let screen = draw(&app, 40);
        assert!(screen.contains("Answers unavailable"), "{screen}");
        // Not asked again while it stays in view, however often the board draws.
        for _ in 0..3 {
            draw(&app, 40);
            app.unread_drawn();
        }
        assert_ne!(
            app.unread_overlay_read().map(|(_, read)| read.key.1),
            Some(OTHER.to_owned()),
            "a failure is not retried in a loop"
        );
        // Away: the viewport shows only the other group; back: asked once more.
        arrive(
            &mut app,
            MEMBER,
            replies(MEMBER, &[("c", &"x\n\n".repeat(80))]),
        );
        // Scrolling is clamped to the last drawn content, so draw the new answer first.
        draw(&app, 14);
        for _ in 0..12 {
            press(&mut app, KeyCode::PageDown);
        }
        draw(&app, 14);
        app.unread_drawn();
        for _ in 0..12 {
            press(&mut app, KeyCode::PageUp);
        }
        draw(&app, 14);
        app.unread_drawn();
        assert_eq!(
            app.unread_overlay_read().map(|(_, read)| read.key.1),
            Some(OTHER.to_owned()),
            "scrolling back asks once more"
        );
    });
}

#[test]
fn closing_drops_a_late_read_and_the_conversation_read_resumes() {
    crate::status::with_now_ms(10_000_000, || {
        let mut app = board();
        install(&mut app, &[(MEMBER, "w", 8_000_000, "final")]);
        app.open_unread(&UnreadScope::Member(MEMBER.into()));
        let (first, read) = app
            .unread_overlay_read()
            .expect("the overlay demands its read");
        assert!(matches!(
            app.selected_read(),
            Some(crate::board::refresh::SelectedRead::UnreadBodies(session, _)) if session == first
        ));
        press(&mut app, KeyCode::Esc);
        assert!(app.unread_overlay.is_none());
        assert!(
            !matches!(
                app.selected_read(),
                Some(crate::board::refresh::SelectedRead::UnreadBodies(..))
            ),
            "the lane is the conversation's again on the next frame"
        );
        // A result of the closed opening finds no overlay.
        app.unread_overlay_arrived(first, &read, replies(MEMBER, &[("w", "Late")]));
        assert!(app.unread_overlay.is_none());
        // A reopened overlay is a new opening: the old one's result is dropped.
        app.open_unread(&UnreadScope::Member(MEMBER.into()));
        let second = app.unread_overlay.as_ref().unwrap().session;
        assert_ne!(first, second);
        app.unread_overlay_arrived(first, &read, replies(MEMBER, &[("w", "Late")]));
        let screen = draw(&app, 40);
        assert!(!screen.contains("Late"), "{screen}");
        assert!(screen.contains("Loading answers"), "{screen}");
    });
}

#[test]
fn u_opens_the_selection_s_unread_and_enter_opens_the_answer_in_chat() {
    crate::status::with_now_ms(10_000_000, || {
        let mut app = app_with_two_members();
        install(
            &mut app,
            &[
                (MEMBER, "w", 8_000_000, "final"),
                (OTHER, "c", 9_000_000, "final"),
            ],
        );
        app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
        app.workspace_unread();
        assert_eq!(
            app.unread_overlay.as_ref().unwrap().members,
            [OTHER, MEMBER]
        );
        app.unread_overlay = None;
        app.workspace.selected = Some(OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: OTHER.into(),
        });
        app.workspace_unread();
        assert_eq!(app.unread_overlay.as_ref().unwrap().members, [OTHER]);
        draw(&app, 40);
        press(&mut app, KeyCode::Enter);
        assert!(app.unread_overlay.is_none());
        assert_eq!(
            app.workspace.selected,
            Some(OpsTarget::Member {
                room_id: Some(ROOM.into()),
                identity_id: OTHER.into(),
            })
        );
        assert_eq!(app.focus, 1);
    });
}

#[test]
fn a_press_inside_the_overlay_never_reaches_the_board_under_it_and_outside_closes_it() {
    crate::status::with_now_ms(10_000_000, || {
        let mut app = app_with_two_members();
        install(
            &mut app,
            &[
                (MEMBER, "w", 8_000_000, "final"),
                (OTHER, "c", 9_000_000, "final"),
            ],
        );
        app.workspace.selected = Some(OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        });
        app.open_unread(&UnreadScope::All);
        draw(&app, 36);
        // Every hit the board painted under the modal, clicked where the modal covers it.
        let outer = app
            .peek_areas
            .dialog(crate::board::app::Overlay::Unread)
            .expect("registered");
        let covered: Vec<_> = app
            .hits
            .borrow()
            .iter()
            .map(|hit| (Rect::new(hit.x, hit.y, hit.width, 1), hit.row()))
            .chain(
                app.workspace_action_hits
                    .borrow()
                    .iter()
                    .map(|(area, _, _)| (*area, None)),
            )
            .filter_map(|(area, row)| {
                let inside = area.intersection(outer);
                (inside.width > 0 && inside.height > 0).then_some((inside, row))
            })
            .collect();
        assert!(!covered.is_empty(), "the board paints hits under the modal");
        let before = (
            app.workspace.selected.clone(),
            app.focus,
            app.menu.is_some(),
        );
        for (area, _) in covered {
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: area.x,
                    row: area.y,
                    modifiers: KeyModifiers::NONE,
                },
                std::time::Instant::now(),
            );
            assert!(app.unread_overlay.is_some(), "{area:?}: still open");
            assert_eq!(
                (
                    app.workspace.selected.clone(),
                    app.focus,
                    app.menu.is_some()
                ),
                before,
                "{area:?}: the press did not act on the board"
            );
        }
        // A press outside closes it and is taken: the board under the pointer does not act.
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 159,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now(),
        );
        assert!(app.unread_overlay.is_none());
        assert_eq!(app.workspace.selected, before.0);
    });
}

#[test]
fn older_answers_held_back_by_core_are_said_so() {
    crate::status::with_now_ms(10_000_000, || {
        let mut app = app_with_two_members();
        let mut page = answers(&[(MEMBER, "w", 8_000_000, "final")]);
        page["nextBefore"] = json!({"answeredAtMs": 7_000_000, "requestId": "older"});
        app.home_leads
            .test_pages(ACTOR, json!({"items":[],"nextBefore":null}), page);
        app.overview_rows.borrow_mut().clear();
        app.open_unread(&UnreadScope::Member(MEMBER.into()));
        arrive(&mut app, MEMBER, replies(MEMBER, &[("w", "The answer")]));
        let screen = draw(&app, 40);
        assert!(screen.contains("The answer"), "{screen}");
        assert!(
            screen.contains("Older unread answers are not loaded"),
            "{screen}"
        );
    });
}

#[test]
fn nothing_unread_shows_the_empty_overlay_without_acknowledgment() {
    let mut app = board();
    install(&mut app, &[]);
    app.workspace_unread();
    assert!(app.unread_overlay.is_some());
    assert!(draw(&app, 40).contains("No unread"));
    assert!(app.unread.writer.offer().is_none());
}

#[test]
fn every_painted_unread_count_opens_the_overlay_for_what_it_counts() {
    crate::status::with_now_ms(10_000_000, || {
        let mut app = app_with_two_members();
        install(
            &mut app,
            &[
                (MEMBER, "w", 8_000_000, "final"),
                (OTHER, "c", 9_000_000, "final"),
            ],
        );
        app.workspace.expanded.insert(ROOM.into());
        app.workspace.selected = Some(OpsTarget::Member {
            room_id: Some(ROOM.into()),
            identity_id: MEMBER.into(),
        });
        draw(&app, 51);
        let hits: Vec<_> = app
            .unread_hits
            .borrow()
            .iter()
            .map(|hit| (hit.area, hit.target.clone()))
            .collect();
        let of = |wanted: &dyn Fn(&Option<OpsTarget>) -> bool| {
            hits.iter().filter(|(_, target)| wanted(target)).count()
        };
        assert_eq!(
            of(&|target| target.is_none()),
            1,
            "the header strip: {hits:?}"
        );
        assert!(
            of(&|target| matches!(target, Some(OpsTarget::Squad(_)))) >= 1,
            "{hits:?}"
        );
        assert!(
            of(
                &|target| matches!(target, Some(OpsTarget::Member { identity_id, .. }) if identity_id == MEMBER)
            ) >= 2,
            "the list row and the conversation header: {hits:?}"
        );
        for (area, target) in &hits {
            let expected: Vec<String> = match target {
                None | Some(OpsTarget::Squad(_)) => vec![OTHER.into(), MEMBER.into()],
                Some(OpsTarget::Member { identity_id, .. }) => vec![identity_id.clone()],
                other => panic!("unexpected target {other:?}"),
            };
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: area.x,
                    row: area.y,
                    modifiers: KeyModifiers::NONE,
                },
                std::time::Instant::now(),
            );
            let overlay = app
                .unread_overlay
                .as_ref()
                .unwrap_or_else(|| panic!("{area:?} {target:?}"));
            assert_eq!(overlay.members, expected, "{area:?} {target:?}");
            press(&mut app, KeyCode::Esc);
            assert!(app.unread_overlay.is_none());
        }
    });
}

#[test]
fn the_composer_chip_opens_the_overlay_and_the_mark_read_action_still_marks() {
    crate::status::with_now_ms(10_000_000, || {
        let mut app = board();
        install(&mut app, &[(MEMBER, "w", 8_000_000, "final")]);
        assert!(app.mark_read(MEMBER), "the explicit action is kept");
        assert!(app.unread.writer.offer().is_some());
    });
}

/// Opt-in local release-mode P02 evidence, never a CI threshold: the overlay open over many
/// unread answers (50 across 10 agents, the most the board holds: `ANSWERS_LIMIT`), with every
/// body loaded, against the conversation view's profile.
#[test]
#[ignore = "local release-mode timing evidence; no CI threshold"]
fn unread_overlay_frame_timing() {
    use crate::board::timing::probe::{measure, report};
    if cfg!(debug_assertions) {
        panic!("run this probe with --release");
    }
    crate::status::with_now_ms(1_791_600_000_000, || {
        let mut app = crate::board::app::workspace::tests::populated_board(1, 10);
        let members: Vec<String> = app
            .workspace_snapshot()
            .unwrap()
            .roster
            .members
            .keys()
            .cloned()
            .collect();
        assert_eq!(members.len(), 10);
        let per_member = 5;
        let mut items = Vec::new();
        for (index, member) in members.iter().enumerate() {
            for n in 0..per_member {
                items.push((
                    member.clone(),
                    format!("req-{index}-{n}"),
                    1_791_000_000_000u64 + (index * per_member + n) as u64 * 1_000,
                ));
            }
        }
        let borrowed: Vec<_> = items
            .iter()
            .map(|(member, request, at)| (member.as_str(), request.as_str(), *at, "final"))
            .collect();
        install(&mut app, &borrowed);
        app.open_unread(&UnreadScope::All);
        for (index, member) in members.iter().enumerate() {
            let texts: Vec<_> = (0..per_member)
                .map(|n| {
                    (
                        format!("req-{index}-{n}"),
                        (0..12)
                            .map(|line| format!("answer {n} line {line} with `code` and **bold**"))
                            .collect::<Vec<_>>()
                            .join("\n\n"),
                    )
                })
                .collect();
            let refs: Vec<_> = texts
                .iter()
                .map(|(request, text)| (request.as_str(), text.as_str()))
                .collect();
            arrive(&mut app, member, replies(member, &refs));
        }
        let rows = items.len();
        let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
        for _ in 0..10 {
            measure(&mut app, &mut terminal, |_, _| {});
        }
        report(
            rows,
            "unread-steady",
            (0..100)
                .map(|_| measure(&mut app, &mut terminal, |_, _| {}))
                .collect(),
        );
        report(
            rows,
            "unread-scroll",
            (0..100)
                .map(|_| {
                    measure(&mut app, &mut terminal, |app, _| {
                        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
                    })
                })
                .collect(),
        );
        report(
            rows,
            "unread-page",
            (0..100)
                .map(|_| {
                    measure(&mut app, &mut terminal, |app, _| {
                        app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
                    })
                })
                .collect(),
        );
    });
}

#[test]
fn every_unavailable_kind_is_admitted_only_when_its_detail_is_selected() {
    crate::status::with_now_ms(10_000_000, || {
        let mut app = app_with_two_members();
        install(
            &mut app,
            &[
                (MEMBER, "missing", 8_000_000, "final"),
                (MEMBER, "unavailable-reply", 8_000_001, "final"),
                (MEMBER, "unavailable-turn", 8_000_002, "turn"),
            ],
        );
        app.open_unread(&UnreadScope::Member(MEMBER.into()));
        arrive(
            &mut app,
            MEMBER,
            unavailable_answers(MEMBER, "unavailable-reply", "unavailable-turn"),
        );
        let screen = draw(&app, 40);
        assert!(screen.contains("answer not loaded"), "{screen}");
        assert_eq!(
            screen.matches("reply text unavailable").count(),
            2,
            "{screen}"
        );
        assert!(
            app.unread.writer.offer().is_none(),
            "paint alone does not write"
        );
        for request in ["missing", "unavailable-reply", "unavailable-turn"] {
            draw(&app, 40);
            app.unread_drawn();
            let requests: Vec<_> = app
                .unread
                .displayed
                .iter()
                .map(|answer| answer.request.as_str())
                .collect();
            assert_eq!(
                requests,
                [request],
                "only the selected unavailable detail grants evidence"
            );
            press(&mut app, KeyCode::Down);
        }
        assert!(app.unread.writer.offer().is_some());
        assert_eq!(
            app.unread_count(MEMBER).total,
            3,
            "Core has not acknowledged them yet"
        );
    });
}

#[test]
fn loading_and_failed_captions_never_admit_unread_answers() {
    for failed in [false, true] {
        let mut app = app_with_two_members();
        install(&mut app, &[(MEMBER, "waiting", 8_000_000, "final")]);
        app.open_unread(&UnreadScope::Member(MEMBER.into()));
        if failed {
            arrive(&mut app, MEMBER, Observation::Unavailable);
        }
        let screen = draw(&app, 40);
        assert!(
            screen.contains(if failed {
                "Answers unavailable"
            } else {
                "Loading answers"
            }),
            "{screen}"
        );
        app.unread_drawn();
        assert!(app.unread.displayed.is_empty());
        assert!(app.unread.writer.offer().is_none());
        assert_eq!(app.unread_count(MEMBER).total, 1);
    }
}

#[test]
fn clearing_a_fifty_answer_page_demands_the_older_metadata_without_acknowledging_it() {
    let mut app = app_with_two_members();
    let requests: Vec<_> = (0..50).map(|n| format!("answer-{n:02}")).collect();
    let items: Vec<_> = requests
        .iter()
        .enumerate()
        .map(|(n, request)| (MEMBER, request.as_str(), 8_000_000 + n as u64, "final"))
        .collect();
    let mut page = answers(&items);
    page["nextBefore"] = json!({"answeredAtMs":8_000_000,"requestId":"answer-00"});
    app.home_leads
        .test_pages(ACTOR, json!({"items":[],"nextBefore":null}), page.clone());
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    arrive(&mut app, MEMBER, replies(MEMBER, &[]));
    for item in page["items"].as_array_mut().unwrap() {
        item["acknowledged"] = json!(true);
    }
    app.home_leads
        .test_pages(ACTOR, json!({"items":[],"nextBefore":null}), page);
    app.unread_overlay_sync();
    assert_eq!(app.unread_count(MEMBER).total, 0);
    assert!(
        app.selected_read().is_some(),
        "a cleared 50-answer page must demand the remaining ten of a 60-answer history"
    );
    assert!(
        app.unread.writer.offer().is_none(),
        "demanding older metadata never acknowledges answers that were not shown"
    );
}

#[test]
fn sixty_unavailable_answers_clear_in_bounded_pages_only_after_their_captions_are_shown() {
    use crate::board::refresh::SelectedRead;
    use crate::board::unread::test_support::Fixture;
    use std::{collections::BTreeSet, fs};
    let fixture = Fixture::new();
    let requests: Vec<_> = (1..=60).rev().map(|n| format!("answer-{n:02}")).collect();
    let rows: Vec<_> = requests
        .iter()
        .enumerate()
        .map(|(n, request)| (MEMBER, request.as_str(), 1000 - n as u64, "final"))
        .collect();
    let mut first = answers(&rows[..50]);
    let before = json!({"answeredAtMs":951,"requestId":"answer-11"});
    first["nextBefore"] = before.clone();
    let older = answers(&rows[50..]);
    fs::write(
        fixture.0.join("pages.json"),
        json!({"":first,"answer-11":older}).to_string(),
    )
    .unwrap();
    let mut app = app_with_two_members();
    app.home_leads
        .test_pages(ACTOR, json!({"items":[],"nextBefore":null}), first);
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    arrive(&mut app, MEMBER, replies(MEMBER, &[]));
    let mut shown = BTreeSet::new();
    let mut continuations = 0;
    for _ in 0..200 {
        draw(&app, 20);
        app.unread_drawn();
        shown.extend(app.unread.displayed.iter().map(|body| body.request.clone()));
        if let Some(writer) = app.unread.writer.offer() {
            let completed = writer.publish(&fixture.core()).unwrap();
            assert!(
                completed
                    .items
                    .iter()
                    .all(|(answer, _)| shown.contains(&answer.request)),
                "an acknowledgment must have a painted caption"
            );
            assert!(completed.items.len() <= 50);
            app.finished_unread(Ok(completed));
        }
        match app.selected_read() {
            Some(SelectedRead::UnreadPage(session, read)) => {
                continuations += 1;
                assert_eq!(read.before, before);
                let acked: Value = serde_json::from_str(
                    &fs::read_to_string(fixture.0.join("acknowledged.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    acked.as_object().unwrap().len(),
                    50,
                    "older metadata waits until the page clears"
                );
                let result = read.read(&fixture.core());
                app.unread_overlay_page_arrived(session, &read, result);
                assert_eq!(
                    app.unread_count(MEMBER).total,
                    10,
                    "the newly known older unread entries appear in the count"
                );
                let acked: Value = serde_json::from_str(
                    &fs::read_to_string(fixture.0.join("acknowledged.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    acked.as_object().unwrap().len(),
                    50,
                    "reading metadata alone never acknowledges"
                );
            }
            Some(SelectedRead::UnreadBodies(_, _)) => {
                arrive(&mut app, MEMBER, replies(MEMBER, &[]))
            }
            _ => {}
        }
        if shown.len() == 60
            && app
                .unread_overlay
                .as_ref()
                .unwrap()
                .items
                .values()
                .all(|answer| answer.acknowledged)
        {
            break;
        }
        press(&mut app, KeyCode::Down);
    }
    let acked: Value =
        serde_json::from_str(&fs::read_to_string(fixture.0.join("acknowledged.json")).unwrap())
            .unwrap();
    assert_eq!(
        acked.as_object().unwrap().len(),
        60,
        "both pages must settle through requests.ack"
    );
    assert_eq!(shown.len(), 60);
    assert_eq!(continuations, 1);
    assert_eq!(app.unread_count(MEMBER).total, 0);
    assert!(app.unread_overlay_page().is_none());
    let calls = fs::read_to_string(fixture.0.join("calls")).unwrap();
    assert!(!calls.contains("requests.show"));
}

#[test]
fn an_unavailable_caption_below_the_viewport_and_a_stale_revision_are_not_admitted() {
    let mut app = app_with_two_members();
    install(
        &mut app,
        &[
            (MEMBER, "long", 8_000_000, "final"),
            (MEMBER, "hidden", 9_000_000, "final"),
        ],
    );
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    arrive(
        &mut app,
        MEMBER,
        replies(MEMBER, &[("long", &"paragraph\n\n".repeat(100))]),
    );
    let screen = draw(&app, 20);
    assert!(!screen.contains("answer not loaded · open it in chat"));
    app.unread_drawn();
    assert_eq!(
        app.unread
            .displayed
            .iter()
            .map(|body| body.request.as_str())
            .collect::<Vec<_>>(),
        ["long"]
    );
    // A new revision published after paint cannot inherit the old body's evidence.
    let mut page = answers(&[
        (MEMBER, "long", 8_000_000, "final"),
        (MEMBER, "hidden", 9_000_000, "final"),
    ]);
    page["items"][0]["revision"] = json!(2);
    app.home_leads
        .test_pages(ACTOR, json!({"items":[],"nextBefore":null}), page);
    app.unread_drawn();
    assert!(app.unread.displayed.is_empty());
    let screen = draw(&app, 20);
    assert!(screen.contains("Loading answers"), "{screen}");
}

#[test]
fn a_failed_continuation_waits_for_reopening_and_a_closed_session_drops_late_pages() {
    let mut app = app_with_two_members();
    let mut page = answers(&[(MEMBER, "cleared", 8_000_000, "final")]);
    page["nextBefore"] = json!({"answeredAtMs":8_000_000,"requestId":"cleared"});
    page["items"][0]["acknowledged"] = json!(true);
    app.home_leads
        .test_pages(ACTOR, json!({"items":[],"nextBefore":null}), page);
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    let (session, read) = app
        .unread_overlay_page()
        .expect("even a cleared head page can continue");
    app.unread_overlay_page_arrived(session, &read, Err("fixture read failed".into()));
    assert!(app.unread_overlay_page().is_none(), "failure must not spin");
    assert!(
        app.notice
            .as_deref()
            .unwrap()
            .contains("reopen Unread to retry")
    );
    press(&mut app, KeyCode::Esc);
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    let (new_session, _) = app.unread_overlay_page().unwrap();
    assert_ne!(session, new_session);
    let older = Page::read(ACTOR, &answers(&[(MEMBER, "late", 7_000_000, "final")])).unwrap();
    app.unread_overlay_page_arrived(session, &read, Ok(older));
    assert!(
        app.unread_overlay.as_ref().unwrap().items.is_empty(),
        "the old session cannot append"
    );
    assert!(app.unread.writer.offer().is_none());
}

#[test]
fn a_new_actor_never_inherits_an_open_overlays_caption_or_cursor() {
    let mut app = app_with_two_members();
    install(&mut app, &[(MEMBER, "old-actor", 8_000_000, "final")]);
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    arrive(&mut app, MEMBER, replies(MEMBER, &[]));
    draw(&app, 40);
    app.view.as_mut().unwrap().me_id = Some(OTHER.into());
    app.unread_drawn();
    assert!(app.unread.displayed.is_empty());
    assert!(app.unread.writer.offer().is_none());
    assert!(app.unread_overlay_page().is_none());
    assert!(app.unread_overlay_read().is_none());
}

#[test]
fn unread_list_detail_uses_the_board_body_instead_of_a_fixed_modal_cap() {
    let mut app = app_with_two_members();
    install(&mut app, &[(MEMBER, "first", 8_000_000, "final")]);
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    arrive(
        &mut app,
        MEMBER,
        replies(MEMBER, &[("first", "The full answer.")]),
    );
    let screen = draw_size(&app, 160, 50);
    let outer = app
        .peek_areas
        .dialog(crate::board::app::Overlay::Unread)
        .unwrap();
    assert_eq!(
        outer.width, 160,
        "Unread fills the board body rather than 110 columns"
    );
    assert!(
        outer.height > 32,
        "Unread uses the available body height: {outer:?}"
    );
    assert!(
        outer.y >= 3,
        "the top bar and tabs remain outside: {outer:?}"
    );
    assert!(screen.contains("Agents"), "{screen}");
    assert!(screen.contains("Digest"), "{screen}");
}

#[test]
fn unread_list_previews_do_not_acknowledge_the_unselected_detail() {
    let mut app = app_with_two_members();
    install(
        &mut app,
        &[
            (MEMBER, "first", 8_000_000, "final"),
            (MEMBER, "second", 9_000_000, "final"),
        ],
    );
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    arrive(
        &mut app,
        MEMBER,
        replies(
            MEMBER,
            &[
                ("first", "First preview.\n\nFirst selected body."),
                ("second", "Second preview.\n\nSecond detail only."),
            ],
        ),
    );
    let screen = draw_size(&app, 160, 50);
    app.unread_drawn();
    assert_eq!(
        app.unread
            .displayed
            .iter()
            .map(|body| body.request.as_str())
            .collect::<Vec<_>>(),
        ["first"],
        "the list preview is not painted-detail evidence"
    );
    assert!(
        screen.contains("Second preview."),
        "the other answer has a list row: {screen}"
    );
    assert!(
        !screen.contains("Second detail only."),
        "the unselected full body stays hidden: {screen}"
    );
    press(&mut app, KeyCode::Down);
    let screen = draw_size(&app, 160, 50);
    app.unread_drawn();
    assert!(screen.contains("Second detail only."), "{screen}");
    assert_eq!(
        app.unread
            .displayed
            .iter()
            .map(|body| body.request.as_str())
            .collect::<Vec<_>>(),
        ["second"]
    );
}

#[test]
fn narrow_unread_stays_unread_until_enter_opens_detail_and_escape_returns_to_list() {
    let mut app = app_with_two_members();
    install(&mut app, &[(MEMBER, "first", 8_000_000, "final")]);
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    arrive(
        &mut app,
        MEMBER,
        replies(MEMBER, &[("first", "Preview only.\n\nFull narrow detail.")]),
    );
    let list = draw_size(&app, 80, 30);
    app.unread_drawn();
    assert!(
        app.unread.displayed.is_empty(),
        "a narrow list preview is never read evidence"
    );
    assert!(list.contains("Preview only."), "{list}");
    assert!(!list.contains("Full narrow detail."), "{list}");
    assert!(app.unread.writer.offer().is_none());
    press(&mut app, KeyCode::Enter);
    assert!(
        app.unread_overlay.is_some(),
        "narrow Enter opens detail inside the overlay"
    );
    let detail = draw_size(&app, 80, 30);
    app.unread_drawn();
    assert!(detail.contains("Full narrow detail."), "{detail}");
    assert_eq!(app.unread.displayed.len(), 1);
    press(&mut app, KeyCode::Esc);
    assert!(
        app.unread_overlay.is_some(),
        "first Escape returns to the narrow list"
    );
    let list = draw_size(&app, 80, 30);
    app.unread_drawn();
    assert!(!list.contains("Full narrow detail."), "{list}");
    assert!(app.unread.displayed.is_empty());
    press(&mut app, KeyCode::Esc);
    assert!(
        app.unread_overlay.is_none(),
        "second Escape closes the overlay"
    );
}

#[test]
fn unread_r_explicitly_marks_the_selected_revision_without_opening_its_body() {
    let mut app = app_with_two_members();
    install(&mut app, &[(MEMBER, "first", 8_000_000, "final")]);
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    draw_size(&app, 80, 30);
    assert!(app.unread.writer.offer().is_none());
    press(&mut app, KeyCode::Char('r'));
    assert!(
        app.unread.writer.offer().is_some(),
        "r admits the selected exact revision even without body text"
    );
    assert_eq!(
        app.unread_count(MEMBER).total,
        1,
        "admission does not claim Core success"
    );
}

#[test]
fn unread_a_requires_one_confirmation_and_escape_cancels_without_acknowledging() {
    let mut app = app_with_two_members();
    install(&mut app, &[(MEMBER, "first", 8_000_000, "final")]);
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    draw_size(&app, 80, 30);
    press(&mut app, KeyCode::Char('a'));
    let screen = draw_size(&app, 80, 30);
    assert!(
        screen.contains("Mark all read"),
        "a asks for confirmation: {screen}"
    );
    assert!(
        app.unread.writer.offer().is_none(),
        "confirmation alone does not acknowledge"
    );
    press(&mut app, KeyCode::Esc);
    assert!(
        app.unread_overlay.is_some(),
        "cancel returns to the overlay"
    );
    assert!(app.unread.writer.offer().is_none());
    assert_eq!(app.unread_count(MEMBER).total, 1);
}

#[test]
fn confirmed_mark_all_walks_one_frozen_scope_past_fifty_without_touching_another_agent() {
    use crate::board::unread::test_support::Fixture;
    use std::fs;
    let fixture = Fixture::new();
    let requests: Vec<_> = (1..=60).rev().map(|n| format!("scope-{n:02}")).collect();
    let rows: Vec<_> = requests
        .iter()
        .enumerate()
        .map(|(n, request)| {
            (
                if n % 2 == 0 { MEMBER } else { OTHER },
                request.as_str(),
                1000 - n as u64,
                "final",
            )
        })
        .collect();
    let mut first = answers(&rows[..20]);
    first["nextBefore"] = json!({"answeredAtMs":981,"requestId":"scope-41"});
    let mut older_rows = rows[20..].to_vec();
    older_rows.push((
        "77777777-7777-4777-8777-777777777777",
        "foreign",
        900,
        "final",
    ));
    fs::write(
        fixture.0.join("pages.json"),
        json!({"":first,"scope-41":answers(&older_rows)}).to_string(),
    )
    .unwrap();
    let mut app = app_with_two_members();
    app.home_leads
        .test_pages(ACTOR, json!({"items":[],"nextBefore":null}), first);
    app.open_unread(&UnreadScope::Group(vec![MEMBER.into(), OTHER.into()]));
    draw_size(&app, 80, 30);
    press(&mut app, KeyCode::Char('a'));
    assert!(draw_size(&app, 80, 30).contains("Mark all read"));
    assert!(
        app.unread.writer.offer().is_none(),
        "no write before confirmation"
    );
    press(&mut app, KeyCode::Enter);
    for _ in 0..10 {
        let Some(writer) = app.unread.writer.offer() else {
            break;
        };
        let completed = writer.publish(&fixture.core()).unwrap();
        assert!(completed.items.len() <= 50);
        assert!(
            completed
                .items
                .iter()
                .all(|(answer, _)| [MEMBER, OTHER].contains(&answer.member.as_str()))
        );
        app.finished_unread(Ok(completed));
    }
    let acked: Value =
        serde_json::from_str(&fs::read_to_string(fixture.0.join("acknowledged.json")).unwrap())
            .unwrap();
    assert_eq!(acked.as_object().unwrap().len(), 60);
    assert!(
        acked.get("foreign").is_none(),
        "an unrelated agent is outside the confirmed scope"
    );
    assert_eq!(
        app.unread_count(MEMBER).total + app.unread_count(OTHER).total,
        0
    );
    let calls = fs::read_to_string(fixture.0.join("calls")).unwrap();
    assert_eq!(
        calls
            .lines()
            .filter(|line| line.contains("requests.list"))
            .count(),
        1,
        "one older-page read, not one walk per member"
    );
    assert!(!calls.contains("requests.show"));
    assert!(app.unread.writer.offer().is_none());
}

#[test]
fn mark_read_is_a_noop_for_a_read_entry_and_retries_a_failed_detail() {
    let mut app = app_with_two_members();
    let mut page = answers(&[(MEMBER, "first", 8_000_000, "final")]);
    app.home_leads
        .test_pages(ACTOR, json!({"items":[],"nextBefore":null}), page.clone());
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    page["items"][0]["acknowledged"] = json!(true);
    app.home_leads
        .test_pages(ACTOR, json!({"items":[],"nextBefore":null}), page);
    app.unread_overlay_sync();
    draw_size(&app, 80, 30);
    press(&mut app, KeyCode::Char('r'));
    assert!(
        app.unread.writer.offer().is_none(),
        "mark read never toggles back to unread"
    );
    arrive(&mut app, MEMBER, Observation::Unavailable);
    draw_size(&app, 160, 40);
    press(&mut app, KeyCode::Char('r'));
    assert_eq!(app.unread_overlay_read().unwrap().1.key.1, MEMBER);
    assert!(
        app.unread.writer.offer().is_none(),
        "retrying detail is a read, not an ack"
    );
}

#[test]
fn detail_re_line_uses_only_the_selected_request_and_viewers_input() {
    let mut app = app_with_two_members();
    install(&mut app, &[(MEMBER, "chosen", 8_000_000, "final")]);
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    let mut document = json!({"identityId":MEMBER,"entries":[
        {"entryId":"wrong","inputBatchId":"same-batch","kind":"input","requestId":"other",
         "sender":{"identityId":ACTOR},"delivery":"delivered","availability":"retained",
         "text":"Wrong nearby prompt","truncated":false,"originalBytes":19},
        {"entryId":"mine","inputBatchId":"same-batch","kind":"input","requestId":"chosen",
         "sender":{"identityId":ACTOR},"delivery":"delivered","availability":"retained",
         "text":"Original question","truncated":false,"originalBytes":17},
        {"entryId":"reply","inputBatchId":"same-batch","kind":"reply","requestId":"chosen",
         "availability":"retained","text":"Exact answer","truncated":false,"originalBytes":12,
         "acknowledgements":[],"acknowledgementsTruncated":false}
    ],"responses":[],"coverage":{"olderDropped":false},"capture":{"enabled":true}});
    arrive(
        &mut app,
        MEMBER,
        Observation::Observed(Arc::new(Answers {
            timeline: Some(timeline::parse(MEMBER, &document).unwrap()),
            ..Default::default()
        })),
    );
    let screen = draw_size(&app, 160, 40);
    assert!(screen.contains("re: Original question"), "{screen}");
    assert!(!screen.contains("Wrong nearby prompt"), "{screen}");
    document["entries"][1]["sender"]["identityId"] = json!(OTHER);
    arrive(
        &mut app,
        MEMBER,
        Observation::Observed(Arc::new(Answers {
            timeline: Some(timeline::parse(MEMBER, &document).unwrap()),
            ..Default::default()
        })),
    );
    let screen = draw_size(&app, 160, 40);
    assert!(screen.contains("re: request text unavailable"), "{screen}");
}

#[test]
fn list_hover_changes_paint_without_selection_or_read_and_stale_hits_are_refused() {
    let mut app = app_with_two_members();
    install(
        &mut app,
        &[
            (MEMBER, "first", 8_000_000, "final"),
            (MEMBER, "second", 9_000_000, "final"),
        ],
    );
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    arrive(
        &mut app,
        MEMBER,
        replies(
            MEMBER,
            &[("first", "First preview"), ("second", "Second preview")],
        ),
    );
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, &app))
        .unwrap();
    let before = terminal.backend().buffer().clone();
    let target = app
        .unread_overlay
        .as_ref()
        .unwrap()
        .list_frame
        .borrow()
        .as_ref()
        .unwrap()
        .geometry
        .iter()
        .find(|row| row.id == "answer-1")
        .unwrap()
        .visible;
    let event = MouseEvent {
        kind: MouseEventKind::Moved,
        column: target.x,
        row: target.y,
        modifiers: KeyModifiers::NONE,
    };
    app.mouse(event, std::time::Instant::now());
    terminal
        .draw(|frame| crate::board::view::render(frame, &app))
        .unwrap();
    assert_ne!(
        &before,
        terminal.backend().buffer(),
        "hover must paint a highlight"
    );
    assert_eq!(
        app.unread_overlay.as_ref().unwrap().selected_index(),
        Some(0)
    );
    assert!(
        app.unread.writer.offer().is_none(),
        "hover and painting alone grant no write"
    );
    app.unread_drawn();
    assert_eq!(
        app.unread
            .displayed
            .iter()
            .map(|body| body.request.as_str())
            .collect::<Vec<_>>(),
        ["first"]
    );
    app.invalidate_frame_geometry();
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            ..event
        },
        std::time::Instant::now(),
    );
    assert_eq!(
        app.unread_overlay.as_ref().unwrap().selected_index(),
        Some(0),
        "stale row geometry cannot select"
    );
}

#[test]
fn a_help_covered_selected_detail_never_grants_read_evidence() {
    let mut app = app_with_two_members();
    install(&mut app, &[(MEMBER, "first", 8_000_000, "final")]);
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    arrive(
        &mut app,
        MEMBER,
        replies(MEMBER, &[("first", "Covered answer")]),
    );
    app.help = true;
    draw_size(&app, 160, 40);
    app.unread_drawn();
    assert!(app.unread.displayed.is_empty());
    assert!(app.unread.writer.offer().is_none());
    app.help = false;
    draw_size(&app, 160, 40);
    app.unread_drawn();
    assert_eq!(app.unread.displayed.len(), 1);
}

#[test]
fn the_99_100_column_boundary_changes_only_the_detail_placement_not_the_selected_answer() {
    let mut app = app_with_two_members();
    install(&mut app, &[(MEMBER, "first", 8_000_000, "final")]);
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    arrive(
        &mut app,
        MEMBER,
        replies(MEMBER, &[("first", "Preview.\n\nDetail-only body.")]),
    );
    let narrow = draw_size(&app, 99, 32);
    app.unread_drawn();
    assert!(!narrow.contains("Detail-only body."), "{narrow}");
    assert!(app.unread.displayed.is_empty());
    let wide = draw_size(&app, 100, 32);
    app.unread_drawn();
    assert!(wide.contains("Detail-only body."), "{wide}");
    assert_eq!(
        app.unread
            .displayed
            .iter()
            .map(|body| body.request.as_str())
            .collect::<Vec<_>>(),
        ["first"]
    );
    draw_size(&app, 100, 7);
    app.unread_drawn();
    assert!(
        app.unread.displayed.is_empty(),
        "a clipped detail with no body rows grants no evidence"
    );
}

#[test]
fn tab_scrolls_the_detail_without_changing_the_list_selection() {
    let mut app = app_with_two_members();
    install(
        &mut app,
        &[
            (MEMBER, "first", 8_000_000, "final"),
            (MEMBER, "second", 9_000_000, "final"),
        ],
    );
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    let text = format!(
        "Preview.\n\n{}",
        (0..90)
            .map(|n| format!("Body paragraph {n}"))
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    arrive(
        &mut app,
        MEMBER,
        replies(MEMBER, &[("first", &text), ("second", "Second preview")]),
    );
    let before = draw_size(&app, 160, 30);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::PageDown);
    let after = draw_size(&app, 160, 30);
    assert_ne!(before, after);
    assert!(
        app.unread_overlay
            .as_ref()
            .unwrap()
            .scroll
            .borrow()
            .offset()
            > 0
    );
    assert_eq!(
        app.unread_overlay.as_ref().unwrap().selected_index(),
        Some(0)
    );
    app.unread_drawn();
    assert_eq!(
        app.unread
            .displayed
            .iter()
            .map(|body| body.request.as_str())
            .collect::<Vec<_>>(),
        ["first"]
    );
    press(&mut app, KeyCode::BackTab);
    press(&mut app, KeyCode::Down);
    draw_size(&app, 160, 30);
    assert_eq!(
        app.unread_overlay.as_ref().unwrap().selected_index(),
        Some(1)
    );
}

#[test]
fn a_clipped_mark_all_confirmation_cannot_write_blindly() {
    let mut app = app_with_two_members();
    install(&mut app, &[(MEMBER, "first", 8_000_000, "final")]);
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    draw_size(&app, 60, 7);
    press(&mut app, KeyCode::Char('a'));
    draw_size(&app, 60, 7);
    press(&mut app, KeyCode::Enter);
    assert!(app.unread.writer.offer().is_none());
    assert!(app.unread_overlay.as_ref().unwrap().confirmation.is_some());
}

#[test]
fn the_selected_answer_demands_older_timeline_pages_until_its_exact_body_is_found() {
    let mut app = app_with_two_members();
    install(&mut app, &[(MEMBER, "older", 8_000_000, "final")]);
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    let Observation::Observed(mut first) = replies(MEMBER, &[]) else {
        panic!("observed fixture")
    };
    Arc::make_mut(&mut first)
        .timeline
        .as_mut()
        .unwrap()
        .next_before = Some("opaque-older-page".into());
    arrive(&mut app, MEMBER, Observation::Observed(first));
    draw_size(&app, 160, 40);
    let (session, read) = app.unread_overlay_read().unwrap();
    assert_eq!(
        read.before,
        Some(recent_responses::Cursor::Timeline(
            "opaque-older-page".into()
        ))
    );
    assert!(
        read.held.is_some(),
        "reuse the existing cursor-chain read owner"
    );
    app.unread_overlay_arrived(
        session,
        &read,
        replies(MEMBER, &[("older", "Exact older answer")]),
    );
    let screen = draw_size(&app, 160, 40);
    assert!(screen.contains("Exact older answer"), "{screen}");
    assert!(
        app.unread_overlay_read().is_none(),
        "the target body stops the older walk"
    );
}

#[test]
fn an_empty_head_with_older_metadata_pending_is_loading_not_definitively_empty() {
    let mut app = app_with_two_members();
    let mut page = answers(&[(MEMBER, "settled", 8_000_000, "final")]);
    page["items"][0]["acknowledged"] = json!(true);
    page["nextBefore"] = json!({"answeredAtMs":8_000_000,"requestId":"settled"});
    app.home_leads
        .test_pages(ACTOR, json!({"items":[],"nextBefore":null}), page);
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    let pending = draw_size(&app, 80, 30);
    assert!(pending.contains("Loading older answers"), "{pending}");
    assert!(!pending.contains("No unread"), "{pending}");
    let (session, read) = app.unread_overlay_page().unwrap();
    app.unread_overlay_page_arrived(session, &read, Err("fixture failure".into()));
    let failed = draw_size(&app, 80, 30);
    assert!(failed.contains("Older answers unavailable"), "{failed}");
    assert!(!failed.contains("No unread"), "{failed}");
    press(&mut app, KeyCode::Char('r'));
    assert!(
        app.unread_overlay_page().is_some(),
        "explicit retry re-admits the continuation"
    );
    assert!(app.unread.writer.offer().is_none());
}

#[test]
fn selection_and_hover_keep_the_theme_unread_foreground() {
    use tmt_cli_style::Role;
    let mut app = app_with_two_members();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .squad_looks
        .clear();
    app.view.as_mut().unwrap().look = crate::look::Look {
        theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::Tmt),
        depth: tmt_cli_style::Depth::TrueColor,
    };
    install(
        &mut app,
        &[
            (MEMBER, "first", 8_000_000, "final"),
            (MEMBER, "second", 9_000_000, "final"),
        ],
    );
    app.open_unread(&UnreadScope::Member(MEMBER.into()));
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, &app))
        .unwrap();
    let rows = app
        .unread_overlay
        .as_ref()
        .unwrap()
        .list_frame
        .borrow()
        .as_ref()
        .unwrap()
        .geometry
        .clone();
    let expected = app.look().role(Role::Unread).fg.unwrap();
    for row in &rows {
        assert_eq!(
            terminal.backend().buffer()[(row.visible.x + 2, row.visible.y)].fg,
            expected
        );
    }
    let second = rows[1].visible;
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Moved,
            column: second.x,
            row: second.y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    terminal
        .draw(|frame| crate::board::view::render(frame, &app))
        .unwrap();
    assert_eq!(
        terminal.backend().buffer()[(second.x + 2, second.y)].fg,
        expected
    );
    press(&mut app, KeyCode::Down);
    assert!(
        app.unread_overlay.as_ref().unwrap().hover.get().is_none(),
        "selection clears the old pointer highlight"
    );
}

#[test]
fn unread_detail_inset_gap_preview_ellipsis_and_confirmation_height_are_consistent() {
    let mut agent = capture_case("unread-overlay-agent", crate::look::Look::default());
    let mut confirm = capture_case("unread-overlay-confirm", crate::look::Look::default());
    for app in [&mut agent, &mut confirm] {
        let text = draw_size(app, 100, 36);
        assert!(text.contains("│ re: request text unavailable"), "{text}");
        assert!(text.contains("Opened the pull reques…"), "{text}");
        let overlay = app.unread_overlay.as_ref().unwrap();
        let built = overlay.built.borrow();
        let built = built.as_ref().unwrap();
        assert_eq!(
            built.body.start, 3,
            "one blank row after the two header rows"
        );
        assert_eq!(built.lines[2].width(), 0);
    }
    let overlay = confirm.unread_overlay.as_ref().unwrap();
    let (yes, no) = overlay
        .confirm_hits
        .get()
        .expect("both confirmation controls paint");
    assert_eq!(yes.y, no.y);
    let text = draw_size(&confirm, 100, 36);
    assert!(text.contains("Mark all read"), "{text}");
}

/// The painted buffer at 160 columns.
fn buffer(app: &App, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(160, height)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn pointer(app: &mut App, column: u16, row: u16) {
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Moved,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
}

#[test]
fn the_top_bar_and_row_unread_counts_highlight_only_under_the_pointer() {
    crate::status::with_now_ms(10_000_000, || {
        let mut app = app_with_two_members();
        install(
            &mut app,
            &[
                (MEMBER, "w", 8_000_000, "final"),
                (OTHER, "c", 9_000_000, "final"),
            ],
        );
        app.workspace.expanded.insert(ROOM.into());
        // A colour look, so the hover has a background to compare whatever NO_COLOR says.
        let look = crate::look::Look {
            theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::Tmt),
            depth: tmt_cli_style::Depth::TrueColor,
        };
        let view = app.view.as_mut().unwrap();
        view.look = look;
        for held in view
            .home
            .as_mut()
            .unwrap()
            .workspace
            .squad_looks
            .values_mut()
        {
            *held = look;
        }
        draw(&app, 51);
        assert_eq!(app.look(), look);
        let hot = tmt_cli_style::theme::screen::chip(
            &look.theme,
            tmt_cli_style::theme::screen::ChipKind::Secondary,
            tmt_cli_style::theme::screen::ChipState::Hot,
            look.depth,
        );
        let hits: Vec<_> = app
            .unread_hits
            .borrow()
            .iter()
            .map(|hit| (hit.area, hit.target.clone()))
            .collect();
        let top = hits.iter().find(|(_, target)| target.is_none()).unwrap().0;
        let row = hits
            .iter()
            .find(|(_, target)| matches!(target, Some(OpsTarget::Member { .. })))
            .unwrap()
            .0;
        let at_rest = buffer(&app, 51);
        for area in [top, row] {
            pointer(&mut app, area.x, area.y);
            let painted = buffer(&app, 51);
            for x in area.left()..area.right() {
                assert_eq!(painted[(x, area.y)].bg, hot.bg.unwrap(), "{area:?}");
            }
            for other in [top, row].into_iter().filter(|other| *other != area) {
                assert_eq!(
                    painted[(other.x, other.y)],
                    at_rest[(other.x, other.y)],
                    "only the count under the pointer: {area:?}"
                );
            }
            if area.x > 0 {
                assert_eq!(
                    painted[(area.x - 1, area.y)],
                    at_rest[(area.x - 1, area.y)],
                    "nothing outside the count"
                );
            }
        }
        pointer(&mut app, 0, 50);
        let left = buffer(&app, 51);
        assert_eq!(
            left[(top.x, top.y)],
            at_rest[(top.x, top.y)],
            "moving off clears it"
        );
        // A press still opens the scope's overlay, and an open overlay turns the hover off.
        pointer(&mut app, row.x, row.y);
        app.mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: row.x,
                row: row.y,
                modifiers: KeyModifiers::NONE,
            },
            std::time::Instant::now(),
        );
        assert!(app.unread_overlay.is_some());
        pointer(&mut app, top.x, top.y);
        assert_eq!(app.unread_hover.get(), None, "no hover under a modal");
    });
}
