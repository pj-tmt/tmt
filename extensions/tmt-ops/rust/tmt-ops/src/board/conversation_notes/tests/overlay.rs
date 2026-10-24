use super::*;
use crate::{
    board::{
        app::{App, Effect, Request, workspace::tests::board},
        workspace::OpsTarget,
    },
    checklist::{Applied, Code, Error, model::Stamp},
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
};

fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn press_with(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> Effect {
    app.key(KeyEvent::new(code, modifiers))
}

fn type_text(app: &mut App, text: &str) {
    for ch in text.chars() {
        press(app, KeyCode::Char(ch));
    }
}

/// A board with a member selected whose list Core has answered with these items.
fn read_with(items: Vec<ItemView>) -> (App, Key) {
    let mut app = board();
    let index = app
        .workspace_rows()
        .iter()
        .position(|row| matches!(row.target, OpsTarget::Member { .. }))
        .expect("the fixture has a member");
    app.select(index);
    let key = app.conversation_notes_key().expect("a squad member");
    app.notes = State::reading();
    let now = Instant::now();
    let task = app.notes.offer(Some(&key), now).unwrap();
    let answer = Completed {
        key: task.key.clone(),
        epoch: task.epoch,
        outcome: Outcome::Read(Ok(preview(items))),
    };
    app.finished_notes(answer, now);
    (app, key)
}

fn opened(items: Vec<ItemView>) -> (App, Key) {
    let (mut app, key) = read_with(items);
    press(&mut app, KeyCode::Char('N'));
    assert!(app.notes_overlay.is_some(), "N opens the list");
    (app, key)
}

fn link(n: u8) -> ItemView {
    view(n, Some("https://example.com/a"), Completion::Open, false)
}
fn todo(n: u8) -> ItemView {
    view(n, None, Completion::Open, false)
}
fn done(n: u8) -> ItemView {
    view(n, None, Completion::Complete, false)
}

fn write_of(effect: Effect) -> (Task, Write) {
    let Effect::Notes(task) = effect else {
        panic!("expected a Notes write, got {effect:?}");
    };
    let Work::Write(write) = task.work.clone() else {
        panic!("expected a write, got a read");
    };
    (task, write)
}

fn id(n: u8) -> Id {
    Id::parse(&format!("33333333-3333-4333-8333-3333333333{n:02}")).unwrap()
}

/// Core accepting a write: the list as the test says it now is.
fn accepted(app: &mut App, task: &Task, items: Vec<ItemView>, item: Option<Id>) {
    let applied = Applied {
        current: preview(items).current,
        item_id: item,
        item_revision: None,
        changed: true,
    };
    let answer = Completed {
        key: task.key.clone(),
        epoch: task.epoch,
        outcome: Outcome::Write(Ok(applied)),
    };
    app.finished_notes(answer, Instant::now());
}

fn refused(app: &mut App, task: &Task, error: Error) {
    let answer = Completed {
        key: task.key.clone(),
        epoch: task.epoch,
        outcome: Outcome::Write(Err(error)),
    };
    app.finished_notes(answer, Instant::now());
}

fn paint(app: &App) -> String {
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

#[test]
fn n_is_notes_on_a_member_and_still_creates_a_squad_on_a_squad_row() {
    let (mut app, _) = read_with(vec![todo(1)]);
    assert_eq!(app.bindings()["N"].text, "member-notes");
    assert!(paint(&app).contains("N notes"), "the footer names the verb");
    app.select(0);
    assert_eq!(app.bindings()["N"].text, "create-squad");
    assert!(!paint(&app).contains("N notes"));
}

#[test]
fn clicking_the_chip_opens_the_list_for_the_selected_member() {
    let (mut app, _) = read_with(vec![link(1)]);
    paint(&app);
    let chip = app
        .workspace_action_hits
        .borrow()
        .iter()
        .find(|(_, _, choice)| matches!(choice, crate::board::app::Choice::MemberNotes))
        .map(|(area, _, _)| *area)
        .expect("the chip is a painted hit");
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: chip.x,
            row: chip.y,
            modifiers: KeyModifiers::NONE,
        },
        Instant::now(),
    );
    assert!(app.notes_overlay.is_some());
}

#[test]
fn the_list_shows_links_then_todos_with_who_and_when_and_esc_closes_it() {
    let (mut app, _) = opened(vec![todo(1), link(2), done(3)]);
    let screen = paint(&app);
    assert!(screen.contains("Notes · worker · 3 items"), "{screen}");
    let (links, todos) = (screen.find("Links · 1"), screen.find("Todo · 2"));
    assert!(links.is_some() && todos > links, "{screen}");
    assert!(screen.contains("↗ item 2"), "{screen}");
    assert!(screen.contains("○ item 1"), "{screen}");
    assert!(screen.contains("✓ item 3"), "{screen}");
    press(&mut app, KeyCode::Esc);
    assert!(app.notes_overlay.is_none());
}

#[test]
fn enter_opens_a_link_and_ticks_a_todo_at_the_revision_it_was_read_at() {
    let (mut app, key) = opened(vec![todo(1), link(2)]);
    // The cursor starts on the first item, a link.
    let effect = press(&mut app, KeyCode::Enter);
    assert!(
        matches!(&effect, Effect::Act(Request::Open { link, .. }) if link == "https://example.com/a"),
        "{effect:?}"
    );
    press(&mut app, KeyCode::Down);
    let (_, write) = write_of(press(&mut app, KeyCode::Enter));
    assert_eq!(
        write,
        Write::Toggle {
            item: id(1),
            revision: 1,
            complete: true
        }
    );
    assert!(app.notes.writing(&key));
}

#[test]
fn a_done_todo_is_unticked_by_enter() {
    let (mut app, _) = opened(vec![done(1)]);
    let (_, write) = write_of(press(&mut app, KeyCode::Enter));
    assert!(
        matches!(
            write,
            Write::Toggle {
                complete: false,
                ..
            }
        ),
        "{write:?}"
    );
}

#[test]
fn a_adds_a_todo_or_a_link_and_the_cursor_follows_the_new_item() {
    let (mut app, key) = opened(vec![todo(1)]);
    press(&mut app, KeyCode::Char('a'));
    type_text(&mut app, "write the plan");
    let (task, write) = write_of(press(&mut app, KeyCode::Enter));
    assert_eq!(
        write,
        Write::Add {
            title: "write the plan".into(),
            reference: None,
            inventory: Some(1)
        }
    );
    accepted(&mut app, &task, vec![todo(1), todo(2)], Some(id(2)));
    assert!(app.notes.draft(&key).is_none(), "a sent draft is gone");
    paint(&app);
    let selected = app
        .notes_overlay
        .as_ref()
        .unwrap()
        .ui
        .borrow()
        .picker
        .list
        .selected()
        .map(str::to_owned);
    assert_eq!(selected.as_deref(), Some(id(2).as_str()));

    press(&mut app, KeyCode::Char('a'));
    type_text(&mut app, "https://example.com/pr");
    let (_, write) = write_of(press(&mut app, KeyCode::Enter));
    assert_eq!(
        write,
        Write::Add {
            title: "https://example.com/pr".into(),
            reference: Some("https://example.com/pr".into()),
            inventory: Some(1)
        },
        "a web address becomes a link"
    );
}

#[test]
fn a_link_in_the_second_field_titles_by_the_note() {
    let (mut app, _) = opened(vec![]);
    press(&mut app, KeyCode::Char('a'));
    type_text(&mut app, "design review");
    press(&mut app, KeyCode::Tab);
    type_text(&mut app, "https://example.com/d");
    let (_, write) = write_of(press(&mut app, KeyCode::Enter));
    assert_eq!(
        write,
        Write::Add {
            title: "design review".into(),
            reference: Some("https://example.com/d".into()),
            inventory: Some(1)
        }
    );
}

#[test]
fn esc_keeps_a_typed_draft_and_the_list_reopens_on_it_while_an_empty_one_is_dropped() {
    let (mut app, key) = opened(vec![todo(1)]);
    press(&mut app, KeyCode::Char('a'));
    type_text(&mut app, "half a thought");
    press(&mut app, KeyCode::Esc);
    assert!(app.notes_overlay.is_none());
    assert!(
        app.notes.draft(&key).is_some(),
        "the draft outlives the overlay"
    );

    press(&mut app, KeyCode::Char('N'));
    assert!(paint(&app).contains("half a thought"));
    press(&mut app, KeyCode::Char('!'));
    assert_eq!(app.notes.draft(&key).unwrap().text(0), "half a thought!");
    for _ in 0.."half a thought!".len() {
        press(&mut app, KeyCode::Backspace);
    }
    press(&mut app, KeyCode::Esc);
    assert!(
        app.notes.draft(&key).is_none(),
        "nothing typed, nothing kept"
    );
}

#[test]
fn an_empty_form_is_refused_in_one_line_and_nothing_is_written() {
    let (mut app, key) = opened(vec![]);
    press(&mut app, KeyCode::Char('a'));
    assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
    assert!(paint(&app).contains("Type a note or a link first."));
    assert!(!app.notes.writing(&key));
}

#[test]
fn e_edits_the_chosen_item_and_a_conflict_keeps_the_draft_on_the_new_version() {
    let (mut app, key) = opened(vec![todo(1)]);
    press(&mut app, KeyCode::Char('e'));
    assert_eq!(app.notes.draft(&key).unwrap().text(0), "item 1");
    type_text(&mut app, " (v2)");
    let (task, write) = write_of(press(&mut app, KeyCode::Enter));
    assert_eq!(
        write,
        Write::Edit {
            item: id(1),
            revision: 1,
            title: "item 1 (v2)".into(),
            reference: None
        }
    );
    // Someone else changed the item first: Core refuses with the list as it now stands.
    let mut changed = todo(1);
    changed.item.revision = 4;
    let mut current = preview(vec![changed]).current;
    current.items[0].item.content.title = "item 1 elsewhere".into();
    refused(
        &mut app,
        &task,
        Error {
            code: Code::Conflict,
            message: "stale".into(),
            current: Some(Box::new(current)),
        },
    );
    let screen = paint(&app);
    assert!(
        screen.contains("Changed elsewhere. Your draft is kept"),
        "{screen}"
    );
    assert!(screen.contains("item 1 (v2)"), "the words stay: {screen}");
    assert!(
        screen.contains("item 1 elsewhere"),
        "the list shows the new version"
    );
    // Enter applies the same words to the version it now sees.
    let (_, write) = write_of(press(&mut app, KeyCode::Enter));
    assert!(
        matches!(write, Write::Edit { revision: 4, .. }),
        "re-aimed at the re-read revision: {write:?}"
    );
}

#[test]
fn d_asks_before_deleting_and_esc_keeps_the_item() {
    let (mut app, _) = opened(vec![todo(1)]);
    press(&mut app, KeyCode::Char('d'));
    assert!(paint(&app).contains("Delete “item 1”?"));
    assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
    assert!(app.notes_overlay.is_some(), "Esc only declined the delete");
    press(&mut app, KeyCode::Char('d'));
    let (_, write) = write_of(press(&mut app, KeyCode::Enter));
    assert_eq!(
        write,
        Write::Delete {
            item: id(1),
            revision: 1,
            inventory: 1
        }
    );
}

#[test]
fn shift_arrows_move_an_item_among_its_own_group_in_the_full_order() {
    let (mut app, _) = opened(vec![todo(1), link(2), todo(3), link(4)]);
    // The cursor is on the first link (item 2); moving it down passes the other link only.
    let (_, write) = write_of(press_with(&mut app, KeyCode::Down, KeyModifiers::SHIFT));
    assert_eq!(
        write,
        Write::Reorder {
            inventory: 1,
            order: vec![id(1), id(4), id(3), id(2)]
        }
    );
}

#[test]
fn the_first_of_a_group_does_not_move_up_and_a_second_write_waits_for_the_first() {
    let (mut app, key) = opened(vec![todo(1), todo(2)]);
    assert_eq!(
        press_with(&mut app, KeyCode::Up, KeyModifiers::SHIFT),
        Effect::None
    );
    assert!(!app.notes.writing(&key));
    write_of(press(&mut app, KeyCode::Enter));
    assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
    assert!(paint(&app).contains("Saving the last change…"));
}

#[test]
fn a_retired_members_write_is_shown_in_cores_words() {
    let (mut app, _) = opened(vec![todo(1)]);
    let (task, _) = write_of(press(&mut app, KeyCode::Enter));
    refused(
        &mut app,
        &task,
        Error {
            code: Code::MemberUnavailable,
            message: "The member worker is retired: its conversation list stays readable and takes no writes.".into(),
            current: None,
        },
    );
    assert!(paint(&app).contains("The member worker is retired"));
}

#[test]
fn a_read_that_began_before_a_write_cannot_undo_it() {
    let (mut app, key) = opened(vec![todo(1)]);
    let t1 = Instant::now() + FRESH * 2;
    let read = app.notes.offer(Some(&key), t1).expect("a refresh is due");
    let (task, _) = write_of(press(&mut app, KeyCode::Enter));
    accepted(&mut app, &task, vec![done(1)], Some(id(1)));
    let stale = Completed {
        key: read.key.clone(),
        epoch: read.epoch,
        outcome: Outcome::Read(Ok(preview(vec![todo(1)]))),
    };
    app.finished_notes(stale, t1);
    assert_eq!(
        app.notes.summary(&key),
        Some(&Summary::Counts(Counts {
            links: 0,
            todo: 0,
            done: 1
        }))
    );
}

#[test]
fn a_form_takes_digits_and_the_tab_keys_instead_of_switching_tabs() {
    let (mut app, key) = opened(vec![]);
    press(&mut app, KeyCode::Char('a'));
    type_text(&mut app, "2 1 q ? , / n");
    assert!(app.notes_overlay.is_some(), "no key left the form");
    assert_eq!(app.notes.draft(&key).unwrap().text(0), "2 1 q ? , / n");
}

/// Oracle scenes of the list overlay at a cursor on the second item: a list with links, todos
/// and a done one; an empty list; a refused write on a retired member; and an edit that lost to
/// a change made elsewhere, its words kept.
pub(in crate::board) const LIST_SCENES: &[&str] = &[
    "notes-list",
    "notes-list-empty",
    "notes-list-retired",
    "notes-list-conflict",
];

fn stamped(mut view: ItemView, title: &str, by: &str, ago_ms: u64) -> ItemView {
    view.item.content.title = title.into();
    view.item.added = Some(Stamp {
        by: Id::parse(by).unwrap(),
        at_ms: crate::status::now_ms() - ago_ms,
    });
    view.item.last_changed = view.item.added.clone();
    view
}

pub(in crate::board) fn list_case(scene: &str, look: crate::look::Look) -> App {
    // Ages sit mid-interval, so a slow run still reads the same word.
    const MINUTES: u64 = 150_000;
    const HOURS: u64 = 9_000_000;
    const DAYS: u64 = 129_600_000;
    let items = match scene {
        "notes-list-empty" => vec![],
        _ => vec![
            stamped(link(1), "Notes list design, frame 3", ALICE, HOURS),
            stamped(
                view(
                    2,
                    Some("https://github.com/pj-tmt/tmt/issues/4554"),
                    Completion::Open,
                    false,
                ),
                "Notes list overlay",
                ACTOR,
                MINUTES,
            ),
            stamped(
                view(
                    6,
                    Some("https://documentation.internal.example.org/notes/overlay"),
                    Completion::Open,
                    false,
                ),
                "A title long enough to need truncating at the narrow widths of the board",
                STRANGER,
                DAYS,
            ),
            stamped(todo(3), "Answer the lead about the N key", ACTOR, MINUTES),
            stamped(todo(4), "Check a retired member's list", STRANGER, DAYS),
            stamped(done(5), "Read the conversation list contract", ALICE, HOURS),
        ],
    };
    let (mut app, _) = opened(items);
    app.view.as_mut().unwrap().look = look;
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .squad_looks
        .clear();
    if scene == "notes-list-empty" {
        return app;
    }
    press(&mut app, KeyCode::Down);
    match scene {
        "notes-list-retired" => {
            press(&mut app, KeyCode::Down);
            press(&mut app, KeyCode::Down);
            let (task, _) = write_of(press(&mut app, KeyCode::Enter));
            refused(
                &mut app,
                &task,
                Error {
                    code: Code::MemberUnavailable,
                    message: "The member worker is retired: its conversation list stays readable and takes no writes.".into(),
                    current: None,
                },
            );
        }
        "notes-list-conflict" => {
            press(&mut app, KeyCode::Down);
            press(&mut app, KeyCode::Down);
            press(&mut app, KeyCode::Char('e'));
            type_text(&mut app, " today");
            let (task, _) = write_of(press(&mut app, KeyCode::Enter));
            let mut current = preview(vec![
                stamped(link(1), "Notes list design, frame 3", ALICE, HOURS),
                stamped(
                    todo(3),
                    "Answer the lead about the N key, by noon",
                    ALICE,
                    MINUTES,
                ),
            ])
            .current;
            current.items[1].item.revision = 2;
            refused(
                &mut app,
                &task,
                Error {
                    code: Code::Conflict,
                    message: "stale".into(),
                    current: Some(Box::new(current)),
                },
            );
        }
        _ => {}
    }
    app
}

#[test]
fn the_counter_counts_items_not_headings_and_an_empty_list_has_none() {
    let (app, _) = opened(vec![link(1), todo(2), todo(3), done(4), done(5)]);
    let screen = paint(&app);
    assert!(screen.contains("1–5 of 5"), "{screen}");
    assert!(!screen.contains("of 7"));
    let (app, _) = opened(vec![]);
    let screen = paint(&app);
    assert!(screen.contains("No notes yet"), "{screen}");
    assert!(
        !screen.contains("–1 of 1") && !screen.contains("of 0"),
        "no counter on an empty list\n{screen}"
    );
}
