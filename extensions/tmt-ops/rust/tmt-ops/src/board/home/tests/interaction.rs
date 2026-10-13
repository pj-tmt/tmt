use super::*;
use crate::action::Action;
use crate::board::{
    app::{App, Compose, Effect, Request, Snapshot},
    home::paint,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyCode::*, KeyEvent, KeyModifiers},
    layout::Rect,
    widgets::Paragraph,
};
use tmt_cli_style::breakpoint::{LG, MD};
use unicode_width::UnicodeWidthStr;

pub(super) fn press(app: &mut App, key: KeyCode) -> Effect {
    app.key(KeyEvent::new(key, KeyModifiers::NONE))
}
pub(super) fn board(documents: &[(&str, Value)]) -> App {
    board_resuming(documents, None)
}
/// The HOME board of `documents` as a reloaded process starts it: the carried state is
/// handed over before the first view arrives.
pub(super) fn board_resuming(
    documents: &[(&str, Value)],
    resume: Option<crate::board::resume::Resume>,
) -> App {
    let fixture = Fixture::new("");
    let acquired = fixture.acquired(documents);
    let mut order = documents
        .iter()
        .map(|(name, _)| (*name).into())
        .collect::<Vec<String>>();
    let mut snapshot = crate::board::app::tests::snapshot(ALL, json!([]));
    let view = snapshot.view.as_mut().unwrap();
    view.board = crate::config::Board::simple(
        crate::config::BoardMode::Split,
        crate::config::Direction::LeftRight,
        vec![crate::config::Pane::Rows, crate::config::Pane::Replies],
        &[40, 60],
    );
    view.document = tab_view::all_document(
        &order,
        &acquired.documents,
        &tab_view::tab_attention(&acquired.documents),
    );
    view.home = Some(model(&order, &acquired, 100));
    view.bindings = crate::action::all_preset();
    view.me = Some("ben".into());
    order.push(ALL.into());
    snapshot.tabs = order;
    let mut app = App::new(Some(ALL.into()));
    if let Some(resume) = resume {
        app.carry(resume);
    }
    app.apply(snapshot);
    app
}
pub(super) fn keys(app: &mut App, codes: &[KeyCode]) -> Effect {
    codes.iter().fold(Effect::None, |_, code| press(app, *code))
}

pub(super) fn waiting() -> Value {
    let mut member = row("W", "worker", "blocked");
    member["waitingOnYou"] = json!([{"requestId":"q1","preparedAtMs":20,"preview":"private question one"},{"requestId":"q2","preparedAtMs":30,"preview":"private question two"}]);
    document("a", row("L", "lead-a", "working"), vec![member])
}

#[test]
fn home_footer_keeps_search_after_row_actions_when_it_fits() {
    for width in [160, 100, 80] {
        let hints = paint::hints(width, false);
        assert!(unicode_width::UnicodeWidthStr::width(hints.as_str()) <= width);
        // Talk stays at rank 1; search follows row actions.
        assert!(!hints.contains("t replies"), "{width}: {hints}");
        if width >= 100 {
            assert!(
                hints.starts_with("↑↓ move  ⏎ open  t talk  e expand  / search"),
                "{width}: {hints}"
            );
        }
    }
}

#[test]
fn one_cursor_moves_across_rows_and_sections_and_enter_goes_in() {
    let mut app = board(&[("a", waiting())]);
    app.view
        .as_mut()
        .unwrap()
        .bindings
        .insert("L".into(), Action::parse("jump lead").unwrap());
    assert_eq!(
        press(&mut app, Char('L')),
        Effect::Act(Request::Jump("lead-a".into()))
    );
    assert_eq!(
        press(&mut app, Enter),
        Effect::Act(Request::Jump("worker".into()))
    );
    assert_eq!(press(&mut app, Char('j')), Effect::None);
    assert_eq!(app.home_target.as_ref().unwrap().section, "leads");
    assert_eq!(
        press(&mut app, Enter),
        Effect::Act(Request::Jump("lead-a".into()))
    );
    press(&mut app, Down);
    assert_eq!(app.home_target.as_ref().unwrap().section, "all-leads");
    press(&mut app, Down);
    assert_eq!(app.home_target.as_ref().unwrap().section, "squads");
    assert_eq!(press(&mut app, Enter), Effect::Load("a".into()));
    let mut app = board(&[("a", waiting())]);
    let selected = app.home_target.clone();
    for key in [Tab, Tab, BackTab, BackTab] {
        assert_eq!(press(&mut app, key), Effect::None);
        assert_eq!(
            app.home_target, selected,
            "Tab uses board focus, not row navigation"
        );
    }
    assert!(!paint::hints(200, true).contains("tab section"));
    let navigation = crate::board::help::model(&app).sections.remove(0);
    assert!(
        navigation
            .entries
            .iter()
            .all(|entry| !entry.description.contains("previous section"))
    );
    assert!(
        navigation
            .entries
            .iter()
            .any(|entry| entry.keys == "↑↓ / j k" && entry.description.contains("cron"))
    );
    press(&mut app, Down);
    assert_eq!(app.home_target.as_ref().unwrap().section, "leads");
    press(&mut app, Up);
    assert_eq!(app.home_target, selected);
    assert_eq!(
        press(&mut app, Enter),
        Effect::Act(Request::Jump("worker".into()))
    );

    app.view
        .as_mut()
        .unwrap()
        .bindings
        .insert("tab".into(), Action::parse("refresh").unwrap());
    assert_eq!(
        press(&mut app, Tab),
        Effect::Refresh,
        "explicit board binding still wins"
    );
    for key in [Char('r'), Char('R'), Char('1')] {
        assert_eq!(press(&mut app, key), Effect::None);
        assert!(app.input.is_none());
    }
    let mut quiet = waiting();
    quiet["sections"][0]["rows"][0]["waitingOnYou"] = json!([]);
    assert_eq!(
        board(&[("a", quiet)]).home_target.unwrap().section,
        "blocked"
    );
}

fn opening_section(doc: Value) -> String {
    board(&[("a", doc)]).home_target.unwrap().section
}

#[test]
fn home_opens_on_the_first_row_from_the_top() {
    // A blocked row wins over a lead, whether or not it also waits on the user.
    assert_eq!(opening_section(waiting()), "blocked");
    let mut blocked = waiting();
    blocked["sections"][0]["rows"][0]["waitingOnYou"] = json!([]);
    assert_eq!(opening_section(blocked), "blocked");
    // A member that only waits on the user has no section of its own: the
    // counts row and the tab marks carry it.
    let mut asking = document(
        "a",
        row("L", "lead-a", "working"),
        vec![row("W", "worker", "working")],
    );
    asking["sections"][0]["rows"][0]["waitingOnYou"] = json!([{"requestId":"q","preparedAtMs":20}]);
    assert_eq!(opening_section(asking), "leads");
    // Nothing needs attention: the first lead, not the squad tile below it.
    let quiet = document(
        "a",
        row("L", "lead-a", "working"),
        vec![row("W", "worker", "working")],
    );
    let app = board(&[("a", quiet)]);
    let target = app.home_target.as_ref().unwrap();
    assert_eq!(
        (target.section.as_str(), target.member.as_deref()),
        ("leads", Some("L"))
    );
    assert_eq!(app.selected, 1);
    // No lead at all: the first squad.
    let leaderless = document("a", Value::Null, vec![row("W", "worker", "working")]);
    assert_eq!(opening_section(leaderless), "squads");
}

#[test]
fn the_first_lead_read_replaces_the_opening_row_once_and_never_after_the_user_moves() {
    let two_leads = || {
        let mut app = board(&[
            ("a", document("a", row("A", "lead-a", "working"), vec![])),
            ("b", document("b", row("B", "lead-b", "working"), vec![])),
        ]);
        let view = app.view.as_mut().unwrap();
        view.me_id = Some("user".into());
        app.home_leads
            .reconcile(view.home.as_ref().unwrap(), view.me_id.as_deref());
        app
    };
    // The read reorders the leads: the asking lead sorts first.
    let reordered = |app: &App, squad: &str| {
        let mut leads = app.home_leads.leads.clone();
        let asking = leads.iter_mut().find(|lead| lead.squad == squad).unwrap();
        asking.exchange = Some(crate::board::home_leads::LeadPreview {
            kind: crate::board::home_leads::Kind::Question,
            request: Some("q".into()),
            since_ms: Some(20),
            preview: "Approve?".into(),
            status: "retained".into(),
        });
        crate::board::home_leads::Read::for_test("user", leads)
    };
    let mut app = two_leads();
    assert_eq!(app.home_target.as_ref().unwrap().squad, "a");
    let read = reordered(&app, "b");
    app.apply_home_leads(read);
    assert_eq!(app.home_target.as_ref().unwrap().squad, "b");
    assert_eq!(
        app.selected, 1,
        "the cursor stays on the first lead, after overview"
    );
    // Only the first read places it: later reads follow the cursor's lead.
    let mut later = app.home_leads.leads.clone();
    later.reverse();
    app.apply_home_leads(crate::board::home_leads::Read::for_test("user", later));
    assert_eq!(app.home_target.as_ref().unwrap().squad, "b");
    // A cursor the user moved stays on its lead.
    let mut app = two_leads();
    press(&mut app, Down);
    assert_eq!(app.home_target.as_ref().unwrap().squad, "b");
    let read = reordered(&app, "a");
    app.apply_home_leads(read);
    assert_eq!(app.home_target.as_ref().unwrap().squad, "b");
    assert_eq!(app.selected, 2);
}

fn leave_and_return(app: &mut App) {
    assert_eq!(app.go("a".into()), Effect::Load("a".into()));
    let mut squad = crate::board::app::tests::snapshot("a", json!([]));
    squad.tabs = app.tabs.clone();
    app.apply(squad);
    assert_eq!(app.current.as_deref(), Some("a"));
    assert!(app.view.as_ref().unwrap().home.is_none());
    assert_eq!(app.go(ALL.into()), Effect::Load(ALL.into()));
}

#[test]
fn switching_back_to_home_returns_to_the_row_that_was_left() {
    let mut doc = waiting();
    doc["sections"][0]["rows"]
        .as_array_mut()
        .unwrap()
        .push(row("Z", "zebra", "working"));
    let mut app = board(&[("a", doc)]);
    keys(&mut app, &[Down]);
    let left = app.home_target.clone();
    assert_eq!(left.as_ref().unwrap().section, "leads");
    leave_and_return(&mut app);
    assert_eq!(app.home_target, left, "restored from the cached view");
    assert_eq!(app.selected, 2);
    // A second round trip keeps the new position, and so does a refresh.
    press(&mut app, Down);
    let left = app.home_target.clone();
    leave_and_return(&mut app);
    assert_eq!(app.home_target, left);
}

#[test]
fn leaving_home_for_a_squad_that_has_not_loaded_and_coming_back_keeps_the_cursor() {
    let mut app = board(&[("a", waiting())]);
    keys(&mut app, &[Down, Down]);
    let left = app.home_target.clone();
    assert_eq!(app.go("a".into()), Effect::Load("a".into()));
    assert_eq!(app.go(ALL.into()), Effect::Load(ALL.into()));
    assert_eq!(app.home_target, left);
}

#[test]
fn refresh_and_search_reconcile_the_stable_target() {
    let mut doc = waiting();
    let second = row("Z", "zebra", "blocked");
    doc["sections"][0]["rows"]
        .as_array_mut()
        .unwrap()
        .push(second);
    let mut app = board(&[("a", doc.clone())]);
    press(&mut app, Down);
    let target = app.home_target.clone();
    let first = row("A", "aardvark", "blocked");
    doc["sections"][0]["rows"]
        .as_array_mut()
        .unwrap()
        .insert(0, first);
    let refreshed = board(&[("a", doc)]);
    app.apply(Snapshot {
        references: Default::default(),
        timing: None,
        squad_keys: vec!["a".into()],
        tabs: app.tabs.clone(),
        hidden: vec![],
        pinned: 0,
        attention: Default::default(),
        squad: Some(ALL.into()),
        view: Ok(refreshed.view.unwrap()),
    });
    assert_eq!(app.home_target, target);
    assert_eq!(app.selected_row().unwrap()["name"], "zebra");
    keys(&mut app, &[Char('/'), Char('z'), Enter]);
    assert_eq!(app.home_target, target);
    press(&mut app, Esc);
    assert_eq!(app.home_target, target);
    app.view = board(&[("a", document("a", Value::Null, vec![]))]).view;
    let view = app.view.as_ref().unwrap();
    app.home_leads
        .reconcile(view.home.as_ref().unwrap(), view.me_id.as_deref());
    press(&mut app, Down);
    assert_eq!(app.selected, 1);
    assert_eq!(app.home_target.unwrap().section, "squads");
}

#[test]
fn answers_use_real_requests_and_submission_rechecks_opening_authority() {
    let mut app = board(&[("a", waiting())]);
    press(&mut app, Char('a'));
    assert_eq!(
        app.menu.as_ref().unwrap().entries[1].label,
        "private question two"
    );
    keys(&mut app, &[Down, Enter, Char('y')]);
    assert_eq!(
        press(&mut app, Enter),
        Effect::Act(Request::Reply {
            me: "ben".into(),
            request: "q2".into(),
            from: "worker".into(),
            text: "y".into()
        })
    );
    for change in ["request", "sender", "loading"] {
        let mut app = board(&[("a", waiting())]);
        keys(&mut app, &[Char('a'), Enter, Char('y')]);
        match change {
            "request" => {
                app.view.as_mut().unwrap().home.as_mut().unwrap().sections[0].rows[0].member["waitingOnYou"] =
                    json!([])
            }
            "sender" => app.view.as_mut().unwrap().me = Some("other".into()),
            _ => app.current = Some("a".into()),
        }
        assert_eq!(press(&mut app, Enter), Effect::None);
        assert!(app.notice.unwrap().contains("nothing sent"));
    }
    let mut app = board(&[("a", waiting())]);
    keys(&mut app, &[Char('a'), Esc]);
    assert!(app.input.is_none() && app.menu.is_none());
    press(&mut app, Char('a'));
    press(&mut app, Enter);
    assert_eq!(press(&mut app, Enter), Effect::None);
    assert_eq!(app.notice.as_deref(), Some("Nothing sent."));
}

#[test]
fn pending_and_squad_notes_use_the_actual_lead_and_refuse_changes() {
    let mut doc = waiting();
    doc["sections"][0]["rows"][0]["waitingOnYou"] = json!([]);
    doc["sections"][0]["rows"][0]["pending"] = json!("note");
    let mut app = board(&[("a", doc.clone())]);
    keys(&mut app, &[Char('a'), Char('x')]);
    assert_eq!(
        press(&mut app, Enter),
        Effect::Act(Request::Annotate {
            me: "ben".into(),
            squad: "a".into(),
            to: "lead-a".into(),
            row: "worker".into(),
            text: "x".into()
        })
    );
    let mut app = board(&[("a", doc)]);
    keys(&mut app, &[End, Char('a')]);
    app.view.as_mut().unwrap().home.as_mut().unwrap().squads[0].lead =
        Some(row("NL", "new-lead", "working"));
    press(&mut app, Char('x'));
    assert_eq!(press(&mut app, Enter), Effect::None);
    let mut app = board(&[("a", document("a", Value::Null, vec![]))]);
    press(&mut app, Char('a'));
    assert!(app.input.is_none());
    assert!(app.notice.as_ref().unwrap().contains("no lead"));
    app.view.as_mut().unwrap().me = None;
    press(&mut app, Char('a'));
    assert!(app.notice.as_ref().unwrap().contains("Who is sending"));
    app.finished(Err("send failed".into()));
    assert_eq!(app.notice.as_deref(), Some("send failed"));
}

#[test]
fn home_tiles_paint_uncovered_known_history_as_partial_at_each_width_and_theme() {
    use crate::board::{
        app::RateView,
        rate::tests::{fixture_seeds, historical_input, history_fixture},
    };
    use crate::config::TokenRate;
    let fixture = history_fixture();
    let input = historical_input(&fixture, false);
    let mut seeds = fixture_seeds(&fixture);
    let offset = crate::status::now_ms() / 5_000 * 5_000 - 25_000;
    let seed = seeds.get_mut("a").unwrap().as_mut().unwrap();
    seed.from += offset;
    seed.through += offset;
    seed.available = seed.available.map(|at| at + offset);
    seed.sampled = seed.sampled.map(|at| at + offset);
    for bucket in &mut seed.buckets {
        bucket.from_ms += offset;
        bucket.to_ms += offset;
        bucket.covered_ms = 0;
        bucket.complete = false;
        bucket.gap = true;
    }
    for width in [160, 100, 80] {
        for (base, depth) in [
            ("tmt", tmt_cli_style::Depth::TrueColor),
            ("tmt-light", tmt_cli_style::Depth::TrueColor),
            ("tmt", tmt_cli_style::Depth::None),
        ] {
            let mut app = board(&[(
                "a",
                document("a", row("a", "product-lead", "working"), vec![]),
            )]);
            let settings = TokenRate {
                enabled: true,
                ..Default::default()
            };
            let view = app.view.as_mut().unwrap();
            view.look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::parse(base).unwrap()),
                depth,
            };
            view.home_rate.insert(
                "a".into(),
                RateView {
                    input: input.clone(),
                    settings,
                    history: Some(seeds.clone()),
                },
            );
            let mut snapshot = crate::board::app::tests::snapshot(ALL, json!([]));
            snapshot.tabs = app.tabs.clone();
            snapshot.view = Ok(app.view.take().unwrap());
            app.apply(snapshot);
            let now = std::time::Instant::now();
            let usage = app.home_usage("a", now).unwrap();
            assert_eq!(usage.lead[2].unwrap().tokens, 21);
            assert!(usage.share.unwrap().partial);
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let screen = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(screen.contains("product-lead"), "{base}/{width}: {screen}");
            let header = terminal
                .backend()
                .buffer()
                .content
                .chunks(width as usize)
                .map(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>())
                .find(|line| line.contains("1h ~21 tok"))
                .expect("longest-window usage line");
            assert!(!header.contains("models "));
            assert!(header.contains("top "), "{base}/{width}: {header}");
            assert!(
                screen.matches("~21").count() >= 2,
                "{base}/{width}: {screen}"
            );
            if width >= 100 {
                assert!(screen.contains("~100%"), "{base}/{width}: {screen}");
            }
        }
    }
}

pub(super) fn header_usage() -> crate::board::app::HomeHeaderUsage<'static> {
    use crate::board::{
        app::{HomeHeaderUsage, UsageModel, UsageShare, UsageTop},
        rate::Reading,
    };
    HomeHeaderUsage {
        windows: crate::config::TokenWindow::DEFAULTS,
        totals: [
            None,
            Some(Reading {
                tokens: 1_200,
                partial: true,
                span: 20_000,
            }),
            Some(Reading {
                tokens: 2_000,
                partial: true,
                span: 20_000,
            }),
        ],
        top: Some(UsageTop {
            member: "worker",
            share: UsageShare {
                fraction: 0.6,
                partial: true,
            },
        }),
        models: vec![
            UsageModel {
                model: Some("gpt-6.1-sol"),
                share: UsageShare {
                    fraction: 0.6,
                    partial: true,
                },
            },
            UsageModel {
                model: Some("claude-opus-4-6"),
                share: UsageShare {
                    fraction: 0.4,
                    partial: true,
                },
            },
        ],
        unreported: 2,
    }
}

#[test]
fn animated_header_boundaries_keep_all_non_counter_cells_and_settled_bytes() {
    use crate::board::{
        home::{bar, counters::Counters},
        view::scene::Kept,
    };
    use ratatui::{buffer::Buffer, layout::Rect};
    use std::time::{Duration, Instant};
    use tmt_cli_style::{Depth, Theme, grid::Align, theme::Base};
    use tmt_tui::components::strip;

    for width in [100, 160] {
        for (base, depth) in [
            ("tmt", Depth::TrueColor),
            ("tmt-light", Depth::TrueColor),
            ("tmt", Depth::None),
        ] {
            let look = crate::look::Look {
                theme: Theme::new(Base::parse(base).unwrap()),
                depth,
            };
            for partial in [false, true] {
                for (from, to) in [
                    (9, 100),
                    (100, 9),
                    (99_000, 110_000),
                    (110_000, 99_000),
                    (998_000, 1_000_000),
                    (1_000_000, 998_000),
                    (9_000_000, 10_000_000),
                    (10_000_000, 9_000_000),
                ] {
                    let now = Instant::now();
                    let mut state = Counters::default();
                    let mut slot = Kept::default();
                    let mut usage = header_usage();
                    usage.totals = [Some(crate::board::rate::Reading {
                        tokens: from,
                        partial,
                        span: 20_000,
                    }); 3];
                    let area = Rect::new(0, 0, width, 1);
                    let draw = |state: &mut Counters,
                                slot: &mut Kept<_>,
                                usage: &crate::board::app::HomeHeaderUsage<'_>,
                                time| {
                        state.begin();
                        let mut digits = std::array::from_fn(|index| {
                            state.digits(
                                format!("header-{index}"),
                                usage.totals[index],
                                false,
                                time,
                            )
                        });
                        for (index, x, budget) in bar::usage_slots(usage, width) {
                            let key = format!("header-{index}");
                            digits[index] = state.admitted_digits(&key, budget);
                            state.visible(
                                &key,
                                Rect::new(x, 0, budget, 1),
                                String::new(),
                                Align::Right,
                                false,
                            );
                        }
                        let line = bar::usage_in(slot, usage, width, look, Some(&digits)).unwrap();
                        let mut buffer = Buffer::empty(area);
                        strip::paint_left(&mut buffer, area, line);
                        state.finish(None, false);
                        buffer
                    };
                    draw(&mut state, &mut slot, &usage, now);
                    usage.totals = usage.totals.map(|reading| {
                        reading.map(|reading| crate::board::rate::Reading {
                            tokens: to,
                            ..reading
                        })
                    });
                    let raw = usage.totals;
                    let start = now + Duration::from_secs(1);
                    let first = draw(&mut state, &mut slot, &usage, start);
                    let builds = slot.builds;
                    let cells = bar::usage_slots(&usage, width);
                    let mut settled = Buffer::empty(area);
                    strip::paint_left(
                        &mut settled,
                        area,
                        paint::usage(&usage, width, look).unwrap(),
                    );
                    if crate::source::tokens(from as f64).len()
                        <= crate::source::tokens(to as f64).len()
                    {
                        assert_ne!(
                            first, settled,
                            "admitted values still animate within their cells"
                        );
                    }
                    for ms in [249, 250, 500, 600] {
                        let time = start + Duration::from_millis(ms);
                        state.tick(time);
                        let frame = draw(&mut state, &mut slot, &usage, time);
                        if ms == 249 {
                            assert_eq!(
                                slot.builds, builds,
                                "unchanged fitted digits reuse the header scene"
                            );
                        }
                        for x in 0..width {
                            if !cells
                                .iter()
                                .any(|&(_, left, cell)| (left..left + cell).contains(&x))
                            {
                                assert_eq!(
                                    frame[(x, 0)],
                                    first[(x, 0)],
                                    "non-counter cell x={x}, {width}/{base}/{partial}: {from}->{to} at {ms}ms"
                                );
                            }
                        }
                        assert_eq!(usage.totals, raw, "paint never changes accepted evidence");
                        if ms == 600 {
                            assert_eq!(
                                frame, settled,
                                "settled bytes and styles retain the existing renderer"
                            );
                        }
                    }
                    assert!(state.wait(start + Duration::from_millis(600)).is_none());
                    // A narrower accepted cell refuses an oversized old display
                    // immediately, rather than truncating it or shifting neighbours.
                    if crate::source::tokens(from as f64).len()
                        > crate::source::tokens(to as f64).len()
                    {
                        assert_eq!(
                            first, settled,
                            "shrinking numeric budgets settle before painting"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn header_usage_formats_thresholds_real_labels_and_partial_missing_values() {
    let app = board(&[]);
    let mut usage = header_usage();
    for width in [160, 100, 80, 60] {
        let line = paint::usage(&usage, width, app.look()).unwrap();
        let text = line.to_string();
        assert!(text.contains("1h ~2k tok"), "{width}: {text}");
        assert_eq!(text.contains("top worker ~60%"), width >= 80);
        assert!(!text.contains("5m") && !text.contains("models") && !text.contains("without data"));
        assert!(line.width() <= usize::from(width));
        assert_eq!(super::glyph_error(&text), None);
        let hidden = crate::board::home::hidden_usage(&usage, width);
        let detail = hidden
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        assert!(detail.contains("5m") && detail.contains("members without data"));
        assert_eq!(detail.contains("worker"), width < 80);
    }
    usage.windows =
        ["5m", "1h", "24h"].map(|text| crate::config::TokenWindow::parse(text).unwrap());
    usage.totals = [None; 3];
    assert!(
        paint::usage(&usage, 160, app.look())
            .unwrap()
            .to_string()
            .contains("24h no reading")
    );
    usage.totals[2] = Some(crate::board::rate::Reading {
        tokens: 0,
        partial: false,
        span: 3_600_000,
    });
    assert!(
        paint::usage(&usage, 160, app.look())
            .unwrap()
            .to_string()
            .contains("24h 0 tok")
    );
}

#[test]
fn flat_header_words_counts_and_missing_evidence_are_text_while_marks_keep_roles() {
    use tmt_cli_style::{Base, Depth, Role, Theme};
    for (base, depth) in [
        (Base::Tmt, Depth::TrueColor),
        (Base::TmtLight, Depth::TrueColor),
        (Base::Terminal, Depth::Ansi16),
        (Base::Tmt, Depth::None),
    ] {
        let app = board(&[(
            "product",
            document(
                "product",
                row("L", "lead", "working"),
                vec![row("W", "worker", "blocked")],
            ),
        )]);
        let look = crate::look::Look {
            theme: Theme::new(base),
            depth,
        };
        let line = paint::summary(app.view.as_ref().unwrap().home.as_ref().unwrap(), 160, look);
        let fg = |role| look.role(role).fg.unwrap_or_default();
        let mut terminal = Terminal::new(TestBackend::new(160, 1)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(Paragraph::new(line), frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        for word in ["Nothing", "blocked", "idle"] {
            let x = text[..text.find(word).unwrap()].width() as u16;
            assert_eq!(buffer[(x, 0)].fg, fg(Role::Text), "{word}");
        }
        let x = text[..text.find("✗").unwrap()].width() as u16;
        assert_eq!(buffer[(x, 0)].fg, fg(Role::Blocked));
        let usage = header_usage();
        let line = paint::usage(&usage, 160, look).unwrap();
        terminal
            .draw(|frame| frame.render_widget(Paragraph::new(line), frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        for word in ["~2k", "worker", "tok"] {
            let x = text[..text.find(word).unwrap()].width() as u16;
            assert_eq!(buffer[(x, 0)].fg, fg(Role::Muted), "{word}");
        }
    }
}

#[test]
fn header_usage_fits_escaped_unicode_names_and_keeps_whole_optional_groups() {
    let app = board(&[]);
    let mut usage = header_usage();
    usage.top.as_mut().unwrap().member = "long-界界界界界界-e\u{301}-worker\nunsafe";
    usage.models[0].model = Some("unknown-model-with-a-very-long-name\u{1b}");
    for width in [MD.cells, LG.cells - 1, LG.cells, 149, 160] {
        let line = paint::usage(&usage, width, app.look()).unwrap();
        let text = line.to_string();
        assert!(line.width() <= width as usize, "{width}: {text}");
        assert!(!text.contains('\n') && !text.contains('\u{1b}'));
        assert!(text.contains("top ") && text.contains("~60%"));
        assert!(!text.contains("members without data"));
        assert!(!text.ends_with(" · "));
    }
}

fn header_frames() -> Value {
    let mut captures = Vec::new();
    for (base, depth, name) in [
        ("tmt", tmt_cli_style::Depth::TrueColor, "tmt"),
        ("tmt-light", tmt_cli_style::Depth::TrueColor, "tmt-light"),
        ("tmt", tmt_cli_style::Depth::None, "NO_COLOR"),
    ] {
        let mut app = board(&[
            ("a", document("a", row("A", "lead-a", "working"), vec![])),
            ("b", document("b", row("B", "lead-b", "working"), vec![])),
        ]);
        app.view.as_mut().unwrap().look = crate::look::Look {
            theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::parse(base).unwrap()),
            depth,
        };
        for width in [160, 100, 80, 160] {
            let mut frames = Vec::new();
            let mut hits = Vec::new();
            let mut buffers = Vec::new();
            for show in [false, true] {
                let line = show
                    .then(|| paint::usage(&header_usage(), width, app.look()))
                    .flatten();
                let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
                terminal
                    .draw(|frame| crate::board::view::render_frame(frame, &app, line, Vec::new()))
                    .unwrap();
                let buffer = terminal.backend().buffer().clone();
                let cells = buffer
                    .content
                    .iter()
                    .map(|cell| {
                        let mut metadata = cell.clone();
                        metadata.set_symbol("");
                        json!({"symbol":cell.symbol(), "style":format!("{metadata:?}")})
                    })
                    .collect::<Vec<_>>();
                let hit = app.hits.borrow().clone();
                frames.push(json!({"cells":cells,"hits":format!("{hit:?}"),
                    "starts":*app.row_starts.borrow(), "tabs":format!("{:?}",app.tab_hits.borrow())}));
                hits.push(hit);
                buffers.push(buffer);
            }
            assert_eq!(hits[0], hits[1], "usage evidence cannot move existing hits");
            assert_eq!(frames[0]["starts"], frames[1]["starts"]);
            assert_eq!(frames[0]["tabs"], frames[1]["tabs"]);
            let usage_y = hits[0].iter().find(|hit| hit.row() == Some(0)).unwrap().y;
            for y in 0..30 {
                if y == usage_y {
                    continue;
                }
                let start = usize::from(y) * usize::from(width);
                let end = start + usize::from(width);
                assert_eq!(
                    &buffers[0].content[start..end],
                    &buffers[1].content[start..end],
                    "only the reserved usage line changes with evidence, {width}/{name}/{y}"
                );
            }
            captures.push(json!({"theme":name,"width":width,"before":frames[0],"after":frames[1]}));
        }
    }
    json!(captures)
}

#[test]
fn home_header_row_preserves_cell_styles_hit_identity_and_resize_order() {
    let frames = header_frames();
    for theme in 0..3 {
        assert_eq!(
            frames[theme * 4],
            frames[theme * 4 + 3],
            "160→100→80→160 restores cells/styles/hits"
        );
    }
}

#[test]
#[ignore = "explicit decoded header evidence, never fixture regeneration"]
fn record_home_header_diff() {
    let path = std::env::var("TMT_HEADER_DIFF_PATH").expect("task-owned evidence output path");
    assert!(std::path::Path::new(&path).starts_with("/private/tmp"));
    fs::write(path, serde_json::to_string(&header_frames()).unwrap()).unwrap();
}

#[test]
fn names_that_are_not_stable_ids_never_reach_the_scene_identity() {
    // Squad and member names are display text: all digits, control characters and
    // very long names must paint, escaped, instead of failing admission.
    let long = "q".repeat(300);
    for squad in ["2024", "tab\there", long.as_str(), " padded "] {
        let mut member = row("W", "wor\nker", "blocked");
        member["waitingOnYou"] = json!([{"requestId":"q","preparedAtMs":20,"preview":"private"}]);
        let app = board(&[(
            squad,
            document(squad, row("L", "lead", "working"), vec![member]),
        )]);
        for width in [80, 100, 160] {
            let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
            terminal
                .draw(|frame| crate::board::view::render(frame, &app))
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(
                text.contains("blocked") && text.contains("Squads"),
                "{squad:?}/{width}"
            );
            assert!(!text.contains('\n') && !text.contains('\t'));
        }
    }
}

#[test]
fn a_reloaded_home_comes_back_on_the_row_it_was_on() {
    let documents = [
        ("a", waiting()),
        (
            "b",
            document(
                "b",
                row("L2", "lead-b", "working"),
                vec![row("M", "member-b", "working")],
            ),
        ),
    ];
    let mut before = board(&documents);
    keys(&mut before, &[Down, Down]);
    let target = before.row_target(before.selected).unwrap();
    let opening = board(&documents);
    assert_ne!(
        opening.row_target(opening.selected),
        Some(target.clone()),
        "the control: a normal start opens elsewhere"
    );

    let resume = before.resume().unwrap();
    let after = board_resuming(&documents, Some(resume));
    assert_eq!(after.row_target(after.selected), Some(target));
    assert!(
        !after.home_start,
        "the opening rule does not move the restored cursor"
    );
}

#[test]
fn overview_is_first_in_traversal_but_does_not_replace_initial_focus() {
    use crate::board::row_detail::Target as DetailTarget;
    for width in [160, 100, 80, 60] {
        let mut app = board(&[("a", waiting())]);
        app.set_body_width(width);
        assert_eq!(
            app.home_entries()[0].target.section,
            crate::board::home::USAGE
        );
        assert_eq!(app.home_target.as_ref().unwrap().section, "blocked");
        assert_eq!(press(&mut app, Up), Effect::None);
        assert_eq!(
            app.home_target.as_ref().unwrap().section,
            crate::board::home::USAGE
        );
        let target = DetailTarget::Row(app.row_target(app.selected).unwrap());
        assert_eq!(press(&mut app, Char('e')), Effect::None);
        assert!(app.row_details.contains(&target));
        assert!(
            app.detail_read().is_none(),
            "overview detail never acquires reply data"
        );
        let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        assert_eq!(press(&mut app, Enter), Effect::None);
        assert!(
            !app.row_details.contains(&target),
            "Enter toggles the same shared detail"
        );
        let stable = app.home_target.clone();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        assert_eq!(
            app.home_target, stable,
            "resize/paint retains the overview identity"
        );
        press(&mut app, Char('/'));
        assert!(
            app.home_entries()
                .iter()
                .all(|entry| entry.target.section != crate::board::home::USAGE)
        );
    }
}

#[test]
fn compact_home_footer_keeps_the_approved_actions_and_discoverable_overflow() {
    for (overflow, expected) in [
        (false, "↑↓  ⏎  t  e  /  ? more  q"),
        (true, "↑↓  ⏎  t  e  /  s  ? more  q"),
    ] {
        assert_eq!(paint::hints(60, overflow), expected);
    }
}

#[test]
fn missing_usage_never_borrows_animated_digits_or_claims_a_token_reading() {
    let mut usage = header_usage();
    usage.totals[2] = None;
    usage.top = None;
    for width in [160, 100, 80, 60] {
        let digits = ["–".into(), "–".into(), "–".into()];
        let line = crate::board::home::bar::usage_in(
            &mut Default::default(),
            &usage,
            width,
            crate::look::Look::default(),
            Some(&digits),
        )
        .unwrap();
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("1h no reading"), "{width}: {text}");
        assert!(!text.contains(" tok") && !text.contains('–'), "{text}");
    }
}
