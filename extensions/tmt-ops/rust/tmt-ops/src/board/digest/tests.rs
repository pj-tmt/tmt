//! Changing a digest from the board: who may, what the dropdown offers, what runs.
use super::*;
use crate::{
    board::{BoardEvent, app::tests::crew, lane},
    cron_service::test_support::{Fixture, LEAD, WORKER},
    labels::{Label, Reader, Supplied, Timing},
    test_support::write_ready_executable,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
};
use serde_json::json;
use std::{
    fs,
    time::{Duration, Instant},
};
use tmt_cli_style::Role;

fn press(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn type_in(app: &mut App, text: &str) {
    for ch in text.chars() {
        press(app, KeyCode::Char(ch));
    }
}

fn choose(current: &str) -> Choose {
    Choose::parse(
        "digest",
        &json!({"kind":"choose","options":[
            {"label":"Default (Auto)","value":"default"},
            {"label":"Auto","value":"auto"},
            {"label":"1m","value":"1m"},
            {"label":"5m","value":"5m"},
            {"label":"10m","value":"10m"},
            {"label":"30m","value":"30m"},
            {"label":"1h","value":"1h"},
            {"label":"20s (custom)","value":"20s"},
            {"label":"Off","value":"off"}],
            "current":current,
            "argv":["digest","auth-fix","--squad","product","{value}"]}),
    )
    .unwrap()
}

fn label(text: &str, action: Option<Choose>) -> Label {
    Label {
        text: text.into(),
        role: Role::Text,
        action,
    }
}

/// A board on `auth-fix` whose digest source supplied a mode chip with a choice.
fn app() -> App {
    let mut app = crew(crate::action::preset(true, &[]), Vec::new());
    supply(&mut app);
    app
}

fn supply(app: &mut App) {
    app.labels = Supplied::from_rows(vec![(
        "digest".into(),
        [(
            "auth-fix".to_owned(),
            vec![
                label("Auto", Some(choose("default"))),
                label("2 held", None),
            ],
        )]
        .into(),
    )]);
}

fn granted(current: &str) -> Granted {
    Granted {
        ask: Ask {
            squad: "product".into(),
            member: "auth-fix".into(),
            choose: choose(current),
        },
        actor: ManagementActor {
            id: "u".into(),
            name: "Ben".into(),
        },
    }
}

fn digest_request(effect: Effect) -> Set {
    match effect {
        Effect::Act(Request::Digest(set)) => set,
        other => panic!("expected a digest change, got {other:?}"),
    }
}

fn menu_labels(app: &App) -> Vec<&str> {
    app.menu
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|entry| entry.label.as_str())
        .collect()
}

/// Moves the dropdown's highlight to the entry whose label starts with `label`.
fn highlight(app: &mut App, label: &str) {
    let target = menu_labels(app)
        .iter()
        .position(|entry| entry.starts_with(label))
        .unwrap_or_else(|| panic!("no entry {label}"));
    while app.menu.as_ref().unwrap().selected != target {
        let key = if app.menu.as_ref().unwrap().selected < target {
            KeyCode::Down
        } else {
            KeyCode::Up
        };
        press(app, key);
    }
}

#[test]
fn the_key_asks_core_before_any_dropdown_opens() {
    let mut app = app();
    let Effect::Act(Request::DigestOpen(ask)) = press(&mut app, KeyCode::Char('i')) else {
        panic!("the key asks for access first");
    };
    assert_eq!(
        (ask.squad.as_str(), ask.member.as_str()),
        ("product", "auth-fix")
    );
    assert_eq!(ask.choose, choose("default"));
    assert!(app.menu.is_none(), "the dropdown waits for the answer");
}

#[test]
fn without_a_choice_to_make_nothing_opens_or_runs() {
    // No extension: nothing supplied labels.
    let mut bare = crew(crate::action::preset(true, &[]), Vec::new());
    assert_eq!(press(&mut bare, KeyCode::Char('i')), Effect::None);
    assert_eq!(
        bare.notice.as_deref(),
        Some("auth-fix has no digest choices to change.")
    );
    // Labels that only display.
    let mut display = app();
    display.labels = Supplied::from_rows(vec![(
        "digest".into(),
        [("auth-fix".to_owned(), vec![label("Auto", None)])].into(),
    )]);
    assert_eq!(press(&mut display, KeyCode::Char('i')), Effect::None);
    assert!(
        display
            .notice
            .as_deref()
            .unwrap()
            .contains("no digest choices")
    );
    // Mid-switch: the rows on screen are another squad's.
    let mut loading = app();
    loading.current = Some("infra".into());
    assert!(loading.loading());
    assert_eq!(press(&mut loading, KeyCode::Char('i')), Effect::None);
    assert_eq!(loading.notice.as_deref(), Some("Loading infra…"));
}

#[test]
fn the_row_menu_lists_the_change_only_for_a_member_whose_source_offers_one() {
    let keys = |app: &mut App| {
        press(app, KeyCode::Enter);
        let keys: Vec<_> = app
            .menu
            .take()
            .unwrap()
            .entries
            .into_iter()
            .map(|entry| (entry.key, entry.label))
            .collect();
        keys
    };
    let bindings = || crate::action::with_action_keys(crate::action::preset(false, &[]));
    let mut offering = crew(bindings(), Vec::new());
    supply(&mut offering);
    let offered = keys(&mut offering);
    assert!(
        offered.contains(&("i".into(), "digest".into())),
        "{offered:?}"
    );
    let mut bare = crew(bindings(), Vec::new());
    assert!(keys(&mut bare).iter().all(|(key, _)| key != "i"));
}

#[test]
fn the_dropdown_lists_the_choices_marks_the_current_one_and_ends_with_custom() {
    let mut app = app();
    app.open_digest(granted("5m"));
    let menu = app.menu.as_ref().unwrap();
    assert_eq!(menu.title, "auth-fix digest");
    assert_eq!(
        menu_labels(&app),
        [
            "Default (Auto)",
            "Auto",
            "1m",
            "5m  (current)",
            "10m",
            "30m",
            "1h",
            "20s (custom)",
            "Off",
            "Custom…"
        ]
    );
    assert_eq!(
        menu.selected, 3,
        "the highlight starts on the current choice"
    );
}

#[test]
fn an_answer_that_arrives_after_something_else_opened_does_not_replace_it() {
    let mut app = app();
    app.digest_custom(granted("5m"));
    assert!(app.input.is_some());
    app.open_digest(granted("5m"));
    assert!(app.menu.is_none() && app.input.is_some());
    app.input = None;
    app.open_digest(granted("5m"));
    app.open_digest(granted("default"));
    assert_eq!(
        app.menu.as_ref().unwrap().selected,
        3,
        "the open menu stays"
    );
}

#[test]
fn keyboard_choose_applies_the_value_as_one_argument_and_esc_changes_nothing() {
    let mut app = app();
    app.open_digest(granted("5m"));
    highlight(&mut app, "1h");
    let set = digest_request(press(&mut app, KeyCode::Enter));
    assert_eq!(set.argv, ["digest", "auth-fix", "--squad", "product", "1h"]);
    assert_eq!(set.shown, "1h");
    assert!(app.menu.is_none());

    app.open_digest(granted("5m"));
    assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
    assert!(app.menu.is_none() && app.input.is_none());
}

#[test]
fn custom_takes_a_duration_and_an_empty_one_goes_back_to_default() {
    let mut app = app();
    app.open_digest(granted("default"));
    highlight(&mut app, "Custom…");
    assert_eq!(press(&mut app, KeyCode::Enter), Effect::None);
    let input = app.input.as_ref().unwrap();
    assert_eq!(input.prompt, "auth-fix digest · Enter sets, Esc cancels");
    assert!(input.hint.as_ref().unwrap().text.contains("20s, 10m or 1h"));
    type_in(&mut app, "20s");
    let set = digest_request(press(&mut app, KeyCode::Enter));
    assert_eq!(set.argv.last().unwrap(), "20s");
    assert!(app.input.is_none());

    // Whatever is typed stays one argument; nothing is split or interpreted.
    app.open_digest(granted("default"));
    highlight(&mut app, "Custom…");
    press(&mut app, KeyCode::Enter);
    type_in(&mut app, "5m --squad other");
    let set = digest_request(press(&mut app, KeyCode::Enter));
    assert_eq!(set.argv.len(), 5);
    assert_eq!(set.argv.last().unwrap(), "5m --squad other");

    // Empty clears the member's own value.
    app.open_digest(granted("5m"));
    highlight(&mut app, "Custom…");
    press(&mut app, KeyCode::Enter);
    let set = digest_request(press(&mut app, KeyCode::Enter));
    assert_eq!(
        (set.argv.last().unwrap().as_str(), set.shown.as_str()),
        ("default", "default")
    );

    // Esc in the input cancels without a request.
    app.open_digest(granted("5m"));
    highlight(&mut app, "Custom…");
    press(&mut app, KeyCode::Enter);
    type_in(&mut app, "1h");
    assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
    assert!(app.input.is_none() && app.menu.is_none());
    assert_eq!(app.notice.as_deref(), Some("Digest not changed."));
}

/// Where `text` is on the drawn board, as a click position.
fn find(app: &App, text: &str) -> (u16, u16) {
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    let buffer = terminal.backend().buffer();
    for y in 0..30 {
        let line: String = (0..80)
            .map(|x| buffer[(x, y)].symbol().to_owned())
            .collect();
        if let Some(at) = line.find(text) {
            return (line[..at].chars().count() as u16, y);
        }
    }
    panic!("{text} is not drawn");
}

fn mouse(kind: MouseEventKind, (column, row): (u16, u16)) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn the_mouse_chooses_an_entry_scrolls_the_highlight_and_ignores_the_outside() {
    let mut app = app();
    app.open_digest(granted("5m"));
    let hour = find(&app, "1h");
    // A click outside the entries keeps the dropdown open.
    assert_eq!(
        app.mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), (0, 0)),
            Instant::now()
        ),
        Effect::None
    );
    assert!(app.menu.is_some());
    // The wheel moves the highlight, one entry per notch, within the list.
    app.mouse(mouse(MouseEventKind::ScrollDown, hour), Instant::now());
    assert_eq!(app.menu.as_ref().unwrap().selected, 4);
    for _ in 0..5 {
        app.mouse(mouse(MouseEventKind::ScrollUp, hour), Instant::now());
    }
    assert_eq!(app.menu.as_ref().unwrap().selected, 0);
    // A click on an entry runs it.
    let hour = find(&app, "1h");
    let set = digest_request(app.mouse(
        mouse(MouseEventKind::Down(MouseButton::Left), hour),
        Instant::now(),
    ));
    assert_eq!(set.argv.last().unwrap(), "1h");
    assert!(app.menu.is_none());
    // Custom… opens the input by the same click.
    app.open_digest(granted("5m"));
    let custom = find(&app, "Custom…");
    assert_eq!(
        app.mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), custom),
            Instant::now()
        ),
        Effect::None
    );
    assert!(app.input.is_some() && app.menu.is_none());
}

fn recording(fixture: &Fixture) -> (Core, std::path::PathBuf) {
    let log = fixture.directory.join("digest.log");
    let wrapper = fixture.directory.join("wrapped");
    write_ready_executable(
        &wrapper,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = digest ]; then\n\
             for a in \"$@\"; do printf '%s\\n' \"$a\"; done >> '{log}'\n\
             echo -- >> '{log}'\n[ -e '{fail}' ] && exit 1\nexit 0\nfi\nexec '{inner}' \"$@\"\n",
            log = log.display(),
            fail = fixture.directory.join("fail").display(),
            inner = fixture.directory.join("tmt").display(),
        ),
    );
    (Core::at(wrapper), log)
}

fn runs(log: &std::path::Path) -> Vec<String> {
    fs::read_to_string(log)
        .map(|text| {
            text.split("--\n")
                .filter(|run| !run.is_empty())
                .map(|run| run.replace('\n', " "))
                .collect()
        })
        .unwrap_or_default()
}

fn ask(member: &str) -> Ask {
    Ask {
        squad: "product".into(),
        member: member.into(),
        choose: choose("default"),
    }
}

#[test]
fn only_the_recorded_user_and_the_squads_lead_may_change_a_digest() {
    let f = Fixture::new();
    // Config::load reads next to the global config; the fixture records Ben there.
    for caller in [None, Some(LEAD)] {
        f.change_model(|m| m["caller"] = json!(caller));
        let granted =
            grant(&f.core, ask("worker")).unwrap_or_else(|error| panic!("{caller:?}: {error}"));
        assert_eq!(granted.ask.member, "worker");
    }
    for caller in [Some(WORKER), Some("ambiguous")] {
        f.change_model(|m| m["caller"] = json!(caller));
        let denied = grant(&f.core, ask("worker")).unwrap_err();
        assert!(
            denied.contains("recorded user or this squad's"),
            "{caller:?}: {denied}"
        );
    }
}

#[test]
fn applying_runs_the_one_literal_argument_and_a_refusal_is_never_claimed_as_saved() {
    let f = Fixture::new();
    let (core, log) = recording(&f);
    let granted = grant(&core, ask("worker")).unwrap();
    let hostile = "5m; touch pwned";
    let set = granted.set(hostile, "custom").unwrap();
    assert_eq!(
        apply(&core, set.clone()).unwrap(),
        "Digest for worker: custom."
    );
    assert_eq!(
        runs(&log),
        [format!("digest auth-fix --squad product {hostile} ")]
    );
    assert!(
        !f.directory.join("pwned").exists(),
        "the value reached a shell"
    );

    fs::write(f.directory.join("fail"), "").unwrap();
    let refused = apply(&core, set).unwrap_err();
    assert_eq!(
        refused,
        "Digest for worker was not changed: custom was not accepted."
    );
    assert_eq!(runs(&log).len(), 2, "a refused change is not repeated");
}

#[test]
fn access_is_checked_again_when_the_choice_is_applied() {
    let f = Fixture::new();
    let (core, log) = recording(&f);
    f.change_model(|m| m["caller"] = json!(LEAD));
    let set = grant(&core, ask("worker"))
        .unwrap()
        .set("1h", "1h")
        .unwrap();
    // The squad's lead changes between opening the dropdown and choosing.
    f.change_model(|m| {
        for member in m["members"].as_array_mut().unwrap() {
            member["metadata"] = json!({});
        }
    });
    let denied = apply(&core, set).unwrap_err();
    assert!(
        denied.contains("recorded user or this squad's lead"),
        "{denied}"
    );
    assert!(runs(&log).is_empty(), "a denied change ran digest");
}

#[test]
fn a_successful_change_asks_the_reader_to_read_again_and_a_refused_one_does_not() {
    let f = Fixture::new();
    let (core, log) = recording(&f);
    let timing = Timing {
        every: Duration::from_secs(60),
        ..Timing::default()
    };
    let reader = Reader::spawn(&core, vec!["digest".into()], timing, |_| true).unwrap();
    let reads = || {
        runs(&log)
            .iter()
            .filter(|run| run.starts_with("digest status"))
            .count()
    };
    eventually("the first read", || reads() == 1);
    let refresh = reader.refresher();
    let set = grant(&core, ask("worker"))
        .unwrap()
        .set("1h", "1h")
        .unwrap();

    fs::write(f.directory.join("fail"), "").unwrap();
    let event = lane::run(
        &core,
        Some(&refresh),
        lane::Job::Act(Request::Digest(set.clone())),
    );
    assert!(matches!(
        event,
        BoardEvent::Acted {
            outcome: Err(_),
            ..
        }
    ));
    fs::remove_file(f.directory.join("fail")).unwrap();
    let event = lane::run(&core, Some(&refresh), lane::Job::Act(Request::Digest(set)));
    assert!(matches!(event, BoardEvent::Acted { outcome: Ok(_), .. }));
    eventually("the read after the change", || reads() >= 2);
    assert_eq!(reads(), 2, "only the successful change read again");
    drop(reader);
}

fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
