use super::*;
use crate::checklist::{
    ItemView,
    model::{Completion, Id, Item, ItemContent},
};

const ROOM: &str = "11111111-1111-4111-8111-111111111111";
const MEMBER: &str = "22222222-2222-4222-8222-222222222222";

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
fn key() -> Key {
    Key {
        room: ROOM.into(),
        member: MEMBER.into(),
    }
}

#[test]
fn a_reference_makes_a_link_and_the_rest_are_todo_or_done_and_archived_counts_nowhere() {
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
            todo: 2,
            done: 1
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
        chip(counts(1, 0, 0).as_ref()),
        "≡ Notes · 1 link · 0 todo · 0 done"
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
        todo: 0,
        done: 0,
    };
    let t1 = t0 + Duration::from_secs(1);
    state.finished(
        Completed {
            key: task.key,
            result: Ok(counts),
        },
        t1,
    );
    assert_eq!(state.summary(&key()), Some(&Summary::Counts(counts)));
    assert!(
        state
            .offer(Some(&key()), t1 + FRESH - Duration::from_millis(1))
            .is_none()
    );
    assert!(
        state.offer(Some(&key()), t1 + FRESH).is_some(),
        "fresh again"
    );
    let other = Key {
        member: "44444444-4444-4444-8444-444444444444".into(),
        ..key()
    };
    assert!(
        state.offer(Some(&other), t1).is_some(),
        "each conversation has its own read"
    );
}

#[test]
fn a_failed_first_read_is_reported_and_retried_later_and_a_failed_refresh_keeps_the_counts() {
    let mut state = State::reading();
    let t0 = Instant::now();
    let task = state.offer(Some(&key()), t0).unwrap();
    state.finished(
        Completed {
            key: task.key,
            result: Err("No admitted caller".into()),
        },
        t0,
    );
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
    state.finished(
        Completed {
            key: task.key.clone(),
            result: Ok(counts),
        },
        t1,
    );
    let task = state.offer(Some(&key()), t1 + FRESH).unwrap();
    state.finished(
        Completed {
            key: task.key,
            result: Err("busy".into()),
        },
        t1 + FRESH,
    );
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
        app.notes.finished(
            Completed {
                key: task.key,
                result,
            },
            now,
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
                todo: 2,
                done: 1,
            }),
        );
        let text = screen(&app);
        assert!(
            text.contains("≡ Notes · 3 links · 2 todo · 1 done"),
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
