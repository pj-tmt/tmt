use super::lines;
use crate::action::Action;
use crate::board::{
    app::{App, Effect, tests::snapshot},
    snapshot_cache::{Display, Rooms},
    view::tests::draw,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const ROOM: &str = "11111111-1111-4111-8111-111111111111";

fn rooms() -> Rooms {
    [
        ("infra", "22222222-2222-4222-8222-222222222222"),
        ("product", ROOM),
    ]
    .into_iter()
    .map(|(name, id)| (name.to_owned(), id.to_owned()))
    .collect()
}

fn row(name: &str, state: &str, task: &str) -> Value {
    json!({"id": format!("id-{name}"), "name": name, "state": state, "fields": {"task": task}})
}

/// A display projected from a published snapshot, as the worker stores it.
fn stored(squad: &str, rows: Value, minutes_old: u64) -> Display {
    let written = crate::status::now_ms() - minutes_old * 60_000;
    Display::project(
        &snapshot(squad, json!([{"title": "Members", "rows": rows}])),
        &rooms(),
        written,
    )
    .expect("a complete snapshot projects")
}

fn opening(squad: &str) -> App {
    let mut app = App::new(Some(squad.into()));
    // Past the spinner delay, so the busy marker shows.
    app.loading_since = Some(Instant::now() - Duration::from_secs(1));
    app
}

#[test]
fn opening_paints_the_stored_rows_with_their_age_until_fresh_rows_replace_them() {
    let mut app = opening("product");
    let rows = json!([row("alice", "working", "ship it"), row("bob", "idle", "")]);
    assert!(app.adopt_cached(stored("product", rows, 2)));
    let screen = draw(&app, 60, 8).join("\n");
    assert!(screen.contains("product · cached 2m ago"), "{screen}");
    assert!(screen.contains("● alice  working  ship it"), "{screen}");
    assert!(screen.contains("○ bob    idle"), "{screen}");

    app.apply(snapshot(
        "product",
        json!([{"title": "Members", "rows": [row("carol", "working", "fresh")]}]),
    ));
    assert!(app.cached_display().is_none());
    let screen = draw(&app, 60, 8).join("\n");
    assert!(!screen.contains("cached"), "{screen}");
    assert!(!screen.contains("alice"), "{screen}");
    assert!(screen.contains("carol"), "{screen}");
}

#[test]
fn no_color_shows_a_steady_busy_marker_instead_of_the_spinner() {
    let mut app = opening("product");
    app.initial_look = Some(crate::look::Look {
        depth: tmt_cli_style::Depth::None,
        ..Default::default()
    });
    assert!(app.adopt_cached(stored("product", json!([row("alice", "working", "")]), 0)));
    let screen = draw(&app, 60, 6).join("\n");
    assert!(
        screen.contains("[busy] product · cached 0s ago"),
        "{screen}"
    );
    // Two later frames: no animation under NO_COLOR.
    app.loading_since = Some(Instant::now() - Duration::from_millis(1_337));
    assert_eq!(draw(&app, 60, 6).join("\n"), screen);
}

#[test]
fn a_stored_display_never_authorizes_live_actions() {
    let mut app = opening("product");
    assert!(app.adopt_cached(stored("product", json!([row("alice", "working", "")]), 1)));
    for verb in ["jump", "talk", "annotate", "copy {name}"] {
        let effect = app.perform(&Action::parse(verb).unwrap());
        assert!(matches!(effect, Effect::None), "{verb}");
        assert_eq!(app.notice.as_deref(), Some("Loading product…"), "{verb}");
    }
}

#[test]
fn only_the_tab_still_waiting_on_its_first_view_adopts_a_stored_display() {
    let mut app = opening("product");
    assert!(
        !app.adopt_cached(stored("infra", json!([]), 1)),
        "another tab"
    );
    app.apply(snapshot(
        "product",
        json!([{"title": null, "rows": [row("a", "", "")]}]),
    ));
    assert!(
        !app.adopt_cached(stored("product", json!([]), 1)),
        "already fresh"
    );

    // A switch keeps the previous tab's rows on screen until a display or view arrives.
    let _ = app.go("infra".into());
    assert!(app.loading());
    assert!(app.adopt_cached(stored("infra", json!([row("dan", "review", "")]), 3)));
    let screen = draw(&app, 60, 8).join("\n");
    assert!(screen.contains("infra · cached 3m ago"), "{screen}");
    assert!(screen.contains("◐ dan"), "{screen}");
    // Leaving the tab drops its display; it never shows under another tab.
    let _ = app.go("product".into());
    assert!(app.cached_display().is_none());
}

#[test]
fn a_refresh_keeps_the_cursor_on_the_same_member_when_rows_reorder() {
    let mut app = opening("product");
    let section = |rows: Value| json!([{"title": null, "rows": rows}]);
    app.apply(snapshot(
        "product",
        section(json!([
            row("a", "", ""),
            row("b", "", ""),
            row("c", "", "")
        ])),
    ));
    app.selected = 1;
    app.apply(snapshot(
        "product",
        section(json!([
            row("c", "", ""),
            row("a", "", ""),
            row("b", "", "")
        ])),
    ));
    assert_eq!(app.rows()[app.selected].1["name"], "b");
    // A member that left keeps the index, clamped like before.
    app.apply(snapshot(
        "product",
        section(json!([row("c", "", ""), row("a", "", "")])),
    ));
    assert_eq!(app.selected, 1);
}

#[test]
fn home_lines_show_squad_counts_and_who_needs_you() {
    let counts = |waiting: u64, blocked: u64| json!({"members": 3, "waiting": waiting, "blocked": blocked, "review": 0, "working": 1, "idle": 1});
    let view = json!({
        "lead": null,
        "sections": [],
        "home": {
            "summary": counts(1, 1),
            "squads": [
                {"squad": "infra", "lead": "ivy", "counts": counts(1, 0), "members": counts(1, 0)},
                {"squad": "product", "lead": null, "counts": counts(0, 1), "members": counts(0, 1)},
            ],
            "sections": [
                {"key": "needs-you", "rows": [{"name": "alice", "squad": "infra", "age": null}]},
                {"key": "blocked", "rows": [{"name": "bob", "squad": "product", "age": null}]},
            ],
        },
    });
    let text: Vec<String> = lines(&view, 80)
        .into_iter()
        .map(|pieces| pieces.into_iter().map(|(text, _)| text).collect())
        .collect();
    assert_eq!(
        text,
        [
            " infra    ◆ 1  ✗ 0  ● 1  ○ 1  lead ivy",
            " product  ◆ 0  ✗ 1  ● 1  ○ 1  ",
            "",
            " Needs you",
            " ◆ alice · infra",
            "",
            " Blocked",
            " ✗ bob · product",
        ]
    );
}

#[test]
fn stored_text_is_escaped_before_painting() {
    let view = json!({"lead": null, "home": null, "sections": [{"title": "t\u{1b}[2J", "rows": [
        {"name": "a\u{7}", "squad": null, "state": null, "task": "x\ny", "waiting": true}
    ]}]});
    let text: String = lines(&view, 80)
        .into_iter()
        .flatten()
        .map(|(text, _)| text)
        .collect();
    assert!(!text.chars().any(|c| c.is_control()), "{text:?}");
    assert!(text.contains('◆'));
}

#[test]
fn home_opens_from_its_stored_display() {
    let mut app = opening(crate::tabs::ALL);
    let counts =
        json!({"members": 1, "waiting": 1, "blocked": 0, "review": 0, "working": 0, "idle": 0});
    let display = Display(json!({
        "version": 1, "writtenAtMs": crate::status::now_ms() - 3_600_000,
        "tabKey": crate::tabs::ALL, "rooms": rooms(), "tabs": [], "hidden": [],
        "pinned": 0, "attention": {},
        "view": {"lead": null, "sections": [], "home": {
            "summary": counts, "squads": [{"squad": "infra", "lead": "ivy", "counts": counts, "members": counts}],
            "sections": [{"key": "needs-you", "rows": [{"name": "alice", "squad": "infra", "age": null}]}],
        }},
    }));
    assert!(app.adopt_cached(display));
    let screen = draw(&app, 60, 8).join("\n");
    assert!(screen.contains("all · cached 1h ago"), "{screen}");
    assert!(screen.contains("infra  ◆ 1"), "{screen}");
    assert!(screen.contains("◆ alice · infra"), "{screen}");
}
