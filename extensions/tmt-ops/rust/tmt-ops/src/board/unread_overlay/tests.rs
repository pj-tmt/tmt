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

fn draw(app: &App, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(160, height)).unwrap();
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
        assert_eq!(app.open_unread(&Scope::All), Effect::None);
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
        assert!(screen.contains("shown answers are marked read"), "{screen}");
    });
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
        app.open_unread(&Scope::Member(MEMBER.into()));
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
        app.open_unread(&Scope::All);
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
        app.open_unread(&Scope::Member(MEMBER.into()));
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
        app.open_unread(&Scope::Member(MEMBER.into()));
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
fn nothing_unread_opens_nothing_and_says_so() {
    let mut app = board();
    install(&mut app, &[]);
    app.workspace_unread();
    assert!(app.unread_overlay.is_none());
    assert_eq!(app.notice.as_deref(), Some("Nothing unread here."));
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
/// unread answers, with every body loaded, against the conversation view's profile.
#[test]
#[ignore = "local release-mode timing evidence; no CI threshold"]
fn unread_overlay_frame_timing() {
    use crate::board::timing::probe::{measure, report};
    if cfg!(debug_assertions) {
        panic!("run this probe with --release");
    }
    crate::status::with_now_ms(1_791_600_000_000, || {
        let mut app = crate::board::app::workspace::tests::populated_board(1, 10);
        let members: Vec<String> = app.workspace_snapshot().unwrap().roster.squads[0]
            .members
            .clone();
        let per_member = 20;
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
        app.open_unread(&Scope::All);
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
