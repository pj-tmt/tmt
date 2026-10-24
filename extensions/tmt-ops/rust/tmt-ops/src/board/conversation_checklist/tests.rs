use super::*;
mod rooms;
use crate::checklist::{
    Current, ItemView, MemberView, Preview, RoomView,
    model::{Completion, Id, Item, ItemContent},
};

const ROOM: &str = "11111111-1111-4111-8111-111111111111";
const MEMBER: &str = "22222222-2222-4222-8222-222222222222";
const ACTOR: &str = "55555555-5555-4555-8555-555555555555";
const ALICE: &str = "66666666-6666-4666-8666-666666666666";
const STRANGER: &str = "77777777-7777-4777-8777-777777777777";

/// Pins an admitted empty-count observation for tests of unrelated state colours/layout.
pub(in crate::board) fn admit_empty(app: &mut crate::board::app::App) {
    let keys = app
        .workspace_snapshot()
        .unwrap()
        .rows
        .keys()
        .filter_map(Key::list)
        .collect::<Vec<_>>();
    let mut state = State::reading();
    state.reconcile(keys.clone().into_iter());
    let now = Instant::now();
    for _ in keys {
        let task = state.offer_background(None, now).unwrap();
        state.finished(
            Completed {
                key: task.key,
                epoch: task.epoch,
                outcome: Outcome::Counts(Ok(Counts::default())),
            },
            now,
        );
    }
    app.notes = state;
}

fn view(n: u8, reference: Option<&str>, completion: Completion, archived: bool) -> ItemView {
    ItemView {
        item: Item {
            id: Id::parse(&format!("33333333-3333-4333-8333-3333333333{n:02}")).unwrap(),
            revision: 1,
            content: ItemContent::new(format!("item {n}"), None, reference.map(str::to_owned))
                .unwrap(),
            assignee: None,
            completion,
            archived,
            added: None,
            last_changed: None,
        },
        assignee_label: None,
        assignee_available: true,
    }
}
fn preview(items: Vec<ItemView>) -> Preview {
    let (room, member) = (Id::parse(ROOM).unwrap(), Id::parse(MEMBER).unwrap());
    Preview {
        actor_id: Id::parse(ACTOR).unwrap(),
        current: Current {
            room: RoomView {
                id: room.clone(),
                name: Some("product".into()),
                available: true,
                manager: true,
            },
            member: Some(MemberView {
                id: member.clone(),
                name: Some("worker".into()),
                retired: false,
            }),
            checklist_id: Some(Id::conversation_checklist(&room, &member)),
            inventory_revision: Some(1),
            items,
            deletion: None,
        },
        members: vec![
            (Id::parse(ACTOR).unwrap(), "lead".into()),
            (Id::parse(ALICE).unwrap(), "alice".into()),
        ],
    }
}

/// A list holding exactly these counts.
fn items_of(counts: Counts) -> Vec<ItemView> {
    assert!(counts.links <= counts.todo + counts.done);
    (0..counts.todo + counts.done)
        .map(|index| {
            view(
                u8::try_from(index + 1).unwrap(),
                (index < counts.links).then_some("https://example.com/a"),
                if index < counts.todo {
                    Completion::Open
                } else {
                    Completion::Complete
                },
                false,
            )
        })
        .collect()
}

/// The answer to a read of this task: a list with these counts, or the words of a failure.
pub(in crate::board) fn reading(task: &Task, result: Result<Counts, String>) -> Completed {
    Completed {
        key: task.key.clone(),
        epoch: task.epoch,
        outcome: Outcome::Read(result.map(|counts| preview(items_of(counts)))),
    }
}

fn key() -> Key {
    Key {
        room: ROOM.into(),
        subject: Subject::Conversation(MEMBER.into()),
    }
}

#[test]
fn references_are_attributes_of_open_and_completed_items_and_archived_counts_nowhere() {
    let items = [
        view(1, Some("https://example.com/a"), Completion::Open, false),
        view(
            2,
            Some("https://example.com/b"),
            Completion::Complete,
            false,
        ),
        view(3, None, Completion::Open, false),
        view(4, None, Completion::Open, false),
        view(5, None, Completion::Complete, false),
        view(6, Some("https://example.com/c"), Completion::Open, true),
        view(7, None, Completion::Open, true),
    ];
    assert_eq!(
        Counts::of(&items),
        Counts {
            links: 2,
            todo: 3,
            done: 2
        }
    );
    assert!(Counts::of(&items[5..]).is_empty(), "only archived items");
    assert!(Counts::of(&[]).is_empty());
}

#[test]
fn the_chip_reads_pending_empty_counts_and_unavailable() {
    let counts = |links, todo, done| Some(Summary::Counts(Counts { links, todo, done }));
    assert_eq!(chip(None), "≡ Notes");
    assert_eq!(chip(Some(&Summary::Pending)), "≡ Notes");
    assert_eq!(chip(counts(0, 0, 0).as_ref()), "≡ Notes · empty");
    assert_eq!(
        chip(counts(3, 2, 1).as_ref()),
        "≡ Notes · 3 links · 2 todo · 1 done"
    );
    assert_eq!(
        chip(counts(1, 1, 0).as_ref()),
        "≡ Notes · 1 link · 1 todo · 0 done"
    );
    assert_eq!(
        chip(Some(&Summary::Unavailable("no".into()))),
        "≡ Notes · unavailable"
    );
}

#[test]
fn a_conversation_is_read_once_then_each_fresh_interval_and_never_twice_at_once() {
    let mut state = State::reading();
    let t0 = Instant::now();
    assert!(
        state.offer(None, t0).is_none(),
        "nothing shown, nothing read"
    );
    let task = state.offer(Some(&key()), t0).expect("the first read");
    assert_eq!(state.summary(&key()), Some(&Summary::Pending));
    assert!(
        state
            .offer(Some(&key()), t0 + Duration::from_secs(60))
            .is_none(),
        "one read in flight at a time"
    );
    let counts = Counts {
        links: 1,
        todo: 1,
        done: 0,
    };
    let t1 = t0 + Duration::from_secs(1);
    state.finished(reading(&task, Ok(counts)), t1);
    assert_eq!(state.summary(&key()), Some(&Summary::Counts(counts)));
    assert!(
        state
            .offer(Some(&key()), t1 + FRESH - Duration::from_millis(1))
            .is_none()
    );
    let refresh = state.offer(Some(&key()), t1 + FRESH).expect("fresh again");
    let other = Key {
        subject: Subject::Conversation("44444444-4444-4444-8444-444444444444".into()),
        ..key()
    };
    assert!(
        state.offer(Some(&other), t1).is_none(),
        "selection cannot queue a second lane read"
    );
    state.finished(reading(&refresh, Ok(counts)), t1 + FRESH);
    assert!(state.offer(Some(&other), t1 + FRESH).is_some());
}

#[test]
fn background_counts_are_serial_minutely_and_do_not_retain_the_list_body() {
    let now = Instant::now();
    let selected = key();
    let other = Key {
        subject: Subject::Conversation(ALICE.into()),
        ..key()
    };
    let mut state = State::reading();
    state.reconcile([selected.clone(), other.clone()].into_iter());
    let read = state.offer(Some(&selected), now).unwrap();
    assert!(state.offer_background(Some(&selected), now).is_none());
    state.finished(
        reading(
            &read,
            Ok(Counts {
                todo: 2,
                ..Counts::default()
            }),
        ),
        now,
    );
    let background = state.offer_background(Some(&selected), now).unwrap();
    assert_eq!(background.key, other);
    assert_eq!(background.work, Work::Counts);
    assert!(state.offer(Some(&selected), now + FRESH).is_none());
    state.finished(
        Completed {
            key: other.clone(),
            epoch: background.epoch,
            outcome: Outcome::Counts(Ok(Counts {
                todo: 3,
                ..Counts::default()
            })),
        },
        now,
    );
    assert!(
        state.preview(&other).is_none(),
        "background retains counts only"
    );
    assert_eq!(state.total_label(), "5 todo");
    assert!(
        state
            .offer_background(Some(&selected), now + BACKGROUND - Duration::from_millis(1))
            .is_none()
    );
    let refresh = state.offer(Some(&selected), now + FRESH).unwrap();
    state.finished(
        reading(
            &refresh,
            Ok(Counts {
                todo: 2,
                ..Counts::default()
            }),
        ),
        now + FRESH,
    );
    assert!(
        state
            .offer_background(Some(&selected), now + BACKGROUND)
            .is_some()
    );
}

#[test]
fn global_totals_deduplicate_conversations_and_keep_missing_observations_unknown() {
    let now = Instant::now();
    let first = key();
    let other = Key {
        room: ALICE.into(),
        ..key()
    };
    let mut state = State::reading();
    assert_eq!(state.total_label(), "… todo");
    state.reconcile([first.clone(), first.clone(), other.clone()].into_iter());
    let read = state.offer(Some(&first), now).unwrap();
    state.finished(
        reading(
            &read,
            Ok(Counts {
                todo: 2,
                ..Counts::default()
            }),
        ),
        now,
    );
    assert_eq!(state.total_label(), "2 todo · partial");
    let read = state.offer(Some(&other), now).unwrap();
    state.finished(reading(&read, Err("unavailable".into())), now);
    assert_eq!(state.total_label(), "2 todo · partial");
    state.reconcile([other].into_iter());
    assert_eq!(state.total_label(), "— todo");
    state.reconcile(std::iter::empty());
    assert_eq!(state.total_label(), "0 todo", "known empty roster");
}

#[test]
fn unread_conversations_are_not_starved_by_an_early_keys_refresh() {
    let now = Instant::now();
    let first = key();
    let later = Key {
        subject: Subject::Conversation(ALICE.into()),
        ..key()
    };
    let mut state = State::reading();
    state.reconcile([first.clone(), later.clone()].into_iter());
    let task = state.offer_background(None, now).unwrap();
    assert_eq!(task.key, first);
    state.finished(
        Completed {
            key: first,
            epoch: task.epoch,
            outcome: Outcome::Counts(Ok(Counts::default())),
        },
        now,
    );
    let next = state.offer_background(None, now + BACKGROUND).unwrap();
    assert_eq!(
        next.key, later,
        "a never-read list precedes an overdue refresh"
    );
}

#[test]
fn a_failed_selected_preview_after_background_counts_waits_before_retrying() {
    let now = Instant::now();
    let key = key();
    let mut state = State::reading();
    state.reconcile([key.clone()].into_iter());
    let task = state.offer_background(None, now).unwrap();
    state.finished(
        Completed {
            key: key.clone(),
            epoch: task.epoch,
            outcome: Outcome::Counts(Ok(Counts {
                todo: 2,
                ..Counts::default()
            })),
        },
        now,
    );
    let selected = state.offer(Some(&key), now).unwrap();
    state.finished(reading(&selected, Err("busy".into())), now);
    assert_eq!(state.total_label(), "2 todo", "retain the admitted counts");
    assert!(
        state
            .offer(Some(&key), now + Duration::from_millis(1))
            .is_none()
    );
    assert!(state.offer(Some(&key), now + FRESH).is_some());
}

#[test]
fn a_removed_conversations_late_read_does_not_leave_a_retained_entry() {
    let now = Instant::now();
    let key = key();
    let mut state = State::reading();
    state.reconcile([key.clone()].into_iter());
    let task = state.offer(Some(&key), now).unwrap();
    state.reconcile(std::iter::empty());
    state.finished(
        reading(
            &task,
            Ok(Counts {
                todo: 2,
                ..Counts::default()
            }),
        ),
        now,
    );
    assert!(state.lists.is_empty());
    assert_eq!(state.total_label(), "0 todo");
}

#[test]
fn a_failed_first_read_is_reported_and_retried_later_and_a_failed_refresh_keeps_the_counts() {
    let mut state = State::reading();
    let t0 = Instant::now();
    let task = state.offer(Some(&key()), t0).unwrap();
    state.finished(reading(&task, Err("No admitted caller".into())), t0);
    assert_eq!(
        state.summary(&key()),
        Some(&Summary::Unavailable("No admitted caller".into()))
    );
    assert!(
        state.offer(Some(&key()), t0 + FRESH).is_none(),
        "a failure waits longer"
    );
    let task = state
        .offer(Some(&key()), t0 + RETRY)
        .expect("then it asks again");

    let counts = Counts {
        links: 0,
        todo: 1,
        done: 0,
    };
    let t1 = t0 + RETRY;
    state.finished(reading(&task, Ok(counts)), t1);
    let task = state.offer(Some(&key()), t1 + FRESH).unwrap();
    state.finished(reading(&task, Err("busy".into())), t1 + FRESH);
    assert_eq!(
        state.summary(&key()),
        Some(&Summary::Counts(counts)),
        "a shown list stays through a failed refresh"
    );
}

#[test]
fn only_a_member_of_a_squad_has_a_list() {
    use crate::board::workspace::OpsTarget;
    let member = |room_id: Option<&str>| OpsTarget::Member {
        room_id: room_id.map(str::to_owned),
        identity_id: MEMBER.into(),
    };
    assert_eq!(Key::of(&member(Some(ROOM))), Some(key()));
    assert_eq!(Key::of(&member(None)), None, "the Common pool has no room");
    assert_eq!(Key::of(&OpsTarget::Squad("product".into())), None);
    assert_eq!(Key::of(&OpsTarget::Pool), None);
}

/// Oracle scenes: the chip once the read has answered with counts, with nothing, and with a failure.
pub(in crate::board) const SCENES: &[&str] = &["notes-counts", "notes-empty", "notes-unavailable"];

pub(in crate::board) fn capture_case(
    scene: &str,
    look: crate::look::Look,
) -> crate::board::app::App {
    let mut app = crate::board::app::workspace::tests::board();
    let view = app.view.as_mut().unwrap();
    view.look = look;
    view.home.as_mut().unwrap().workspace.squad_looks.clear();
    let index = app
        .workspace_rows()
        .iter()
        .position(|row| {
            matches!(
                row.target,
                crate::board::workspace::OpsTarget::Member { .. }
            )
        })
        .expect("the fixture has a member");
    app.select(index);
    let key = app.conversation_notes_key().expect("a squad member");
    if scene == "notes-unavailable" {
        app.notes = State::reading();
        let keys = app
            .workspace_snapshot()
            .unwrap()
            .rows
            .keys()
            .filter_map(Key::list)
            .collect::<Vec<_>>();
        app.notes.reconcile(keys.into_iter());
    } else {
        admit_empty(&mut app);
    }
    let now = Instant::now();
    let task = app.notes.offer(Some(&key), now).unwrap();
    let result = match scene {
        "notes-counts" => Ok(Counts {
            links: 3,
            todo: 5,
            done: 1,
        }),
        "notes-empty" => Ok(Counts::default()),
        _ => Err("No admitted caller or recorded active user.".into()),
    };
    app.notes.finished(reading(&task, result), now);
    if scene == "notes-unavailable" {
        let room = Key::room(&key.room);
        let task = app.notes.offer(Some(&room), now).unwrap();
        app.notes.finished(
            Completed {
                key: task.key,
                epoch: task.epoch,
                outcome: Outcome::Counts(Err("No admitted caller or recorded active user.".into())),
            },
            now,
        );
    }
    app
}

#[test]
fn the_chip_fits_whether_the_ambiguous_equivalence_sign_is_one_cell_or_two() {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    assert_eq!(
        ('≡'.width(), '≡'.width_cjk()),
        (Some(1), Some(2)),
        "the sign is ambiguous"
    );
    let summaries = [
        None,
        Some(Summary::Counts(Counts::default())),
        Some(Summary::Counts(Counts {
            links: 12,
            todo: 345,
            done: 6789,
        })),
        Some(Summary::Unavailable("no".into())),
    ];
    for summary in &summaries {
        for columns in 0..=60 {
            let text = fitted(summary.as_ref(), columns);
            // `columns` is what the header leaves for the chip after its leading space.
            assert!(text.width() <= columns, "{columns}: {text:?}");
            // Counting the sign two cells wide, as an East Asian locale draws it, still fits.
            let sign = usize::from(text.contains('≡'));
            assert!(text.width() + sign <= columns, "{columns}: {text:?} (CJK)");
        }
        let text = chip(summary.as_ref());
        assert_eq!(
            crate::board::glyph_guard::glyph_error(&text),
            None,
            "the sign is a registered mark: {text:?}"
        );
    }
}

mod header {
    use super::*;
    use crate::board::{
        app::{App, workspace::tests::board},
        workspace::OpsTarget,
    };
    use ratatui::{Terminal, backend::TestBackend};

    fn select_member(app: &mut App) -> Key {
        let index = app
            .workspace_rows()
            .iter()
            .position(|row| matches!(row.target, OpsTarget::Member { .. }))
            .expect("the fixture has a member");
        app.select(index);
        app.conversation_notes_key()
            .expect("a squad member has a conversation")
    }
    fn screen(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..40)
            .map(|y| {
                (0..120)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn answer(app: &mut App, key: &Key, result: Result<Counts, String>) {
        app.notes = State::reading();
        let now = Instant::now();
        let task = app.notes.offer(Some(key), now).expect("a read is wanted");
        app.notes.finished(reading(&task, result), now);
    }

    #[test]
    fn rows_show_only_known_positive_todos_and_keep_the_model_alert_header_whole() {
        let mut app = board();
        let key = select_member(&mut app);
        for result in [
            None,
            Some(Err("unavailable".into())),
            Some(Ok(Counts::default())),
            Some(Ok(Counts {
                todo: 2,
                ..Counts::default()
            })),
        ] {
            let positive = matches!(&result, Some(Ok(counts)) if counts.todo > 0);
            if let Some(result) = result {
                answer(&mut app, &key, result);
            }
            let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let rows: Vec<_> = terminal
                .backend()
                .buffer()
                .content
                .chunks(160)
                .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                .collect();
            let member = rows.iter().find(|line| line.contains("└ worker")).unwrap();
            assert_eq!(member.contains("todo"), positive, "{member}");
            assert!(
                rows.iter().any(|line| line.contains("MODEL / ALERT")),
                "{rows:#?}"
            );
        }
    }

    #[test]
    fn todo_counts_reach_the_global_header_and_member_state_together() {
        let mut app = board();
        let key = select_member(&mut app);
        answer(
            &mut app,
            &key,
            Ok(Counts {
                links: 3,
                todo: 5,
                done: 1,
            }),
        );
        let text = screen(&app);
        let header = text.lines().next().unwrap();
        assert!(header.contains("○ 5 todo"), "{header}");
        let member = text
            .lines()
            .find(|line| line.contains("worker") && line.contains("working"))
            .unwrap();
        assert!(member.contains("· 5 todo"), "{member}");
        assert!(!text.contains("need you"), "{text}");
    }

    #[test]
    fn done_publication_updates_every_count_and_a_late_read_cannot_restore_it() {
        let mut app = board();
        let key = select_member(&mut app);
        answer(
            &mut app,
            &key,
            Ok(Counts {
                todo: 2,
                ..Counts::default()
            }),
        );
        let now = Instant::now() + BACKGROUND;
        let old = app.notes.offer_background(None, now);
        // The test seed has no roster scope; admit the same real conversation.
        let old = old
            .or_else(|| {
                app.notes.reconcile([key.clone()].into_iter());
                app.notes.offer_background(None, now)
            })
            .unwrap();
        let item = preview(items_of(Counts {
            todo: 2,
            ..Counts::default()
        }))
        .current
        .items[0]
            .item
            .id
            .clone();
        let write = app
            .notes
            .write(
                &key,
                Write::Toggle {
                    item: item.clone(),
                    revision: 1,
                    complete: true,
                },
            )
            .unwrap();
        app.notes.finished(
            Completed {
                key: key.clone(),
                epoch: write.epoch,
                outcome: Outcome::Write(Ok(crate::checklist::Applied {
                    current: preview(items_of(Counts {
                        todo: 1,
                        done: 1,
                        ..Counts::default()
                    }))
                    .current,
                    item_id: Some(item),
                    item_revision: Some(2),
                    changed: true,
                })),
            },
            now,
        );
        app.notes.finished(
            Completed {
                key,
                epoch: old.epoch,
                outcome: Outcome::Counts(Ok(Counts {
                    todo: 2,
                    ..Counts::default()
                })),
            },
            now,
        );
        let text = screen(&app);
        assert!(text.lines().next().unwrap().contains("○ 1 todo"), "{text}");
        assert!(
            text.contains("≡ Notes · 0 links · 1 todo · 1 done"),
            "{text}"
        );
        assert!(
            text.lines().any(|line| line.contains("worker")
                && line.contains("working")
                && line.contains("· 1 todo")),
            "{text}"
        );
    }

    #[test]
    fn the_member_header_shows_the_notes_summary_where_the_link_line_was() {
        let mut app = board();
        let key = select_member(&mut app);
        let text = screen(&app);
        assert!(
            text.contains("≡ Notes"),
            "the chip stands before the first read answers\n{text}"
        );
        assert!(
            !screen(&app).contains("link —"),
            "the link line is replaced"
        );

        answer(
            &mut app,
            &key,
            Ok(Counts {
                links: 3,
                todo: 5,
                done: 1,
            }),
        );
        let text = screen(&app);
        assert!(
            text.contains("≡ Notes · 3 links · 5 todo · 1 done"),
            "{text}"
        );
    }

    #[test]
    fn empty_and_unavailable_lists_say_so_and_the_loop_asks_for_the_shown_conversation_only() {
        let mut app = board();
        let key = select_member(&mut app);
        answer(&mut app, &key, Ok(Counts::default()));
        assert!(screen(&app).contains("≡ Notes · empty"));

        let mut app = board();
        let key = select_member(&mut app);
        answer(&mut app, &key, Err("no recorded user".into()));
        assert!(screen(&app).contains("≡ Notes · unavailable"));

        let mut app = board();
        let last = app.workspace_rows().len() - 1;
        app.select(0);
        assert_eq!(
            app.conversation_notes_key(),
            None,
            "a squad row shows no member conversation: {:?}",
            app.workspace_rows()[last].target
        );
    }
}

pub(in crate::board) mod overlay;

pub(in crate::board) mod captures;
