//! The provider-limits row inside the whole HOME frame: it follows the usage
//! switch and the width, holds its row before the first sample, and moves
//! nothing else when the sample arrives.
use crate::{
    board::{
        app::{App, RateView},
        limits::{Provider, Reading, Standing, Window},
        rate::Input,
    },
    config::TokenRate,
};
use ratatui::{Terminal, backend::TestBackend};

const NOW: u64 = 1_791_600_000_000;
const HOUR: u64 = 3_600_000;

fn app(usage_enabled: bool) -> App {
    let mut app = crate::board::app::workspace::tests::board();
    crate::board::app::workspace::tests::observe_waiting_member(&mut app);
    app.shell.tab = crate::board::shell::OpsTab::Overview;
    let view = app.view.as_mut().unwrap();
    view.home_rate.insert(
        "product".into(),
        RateView {
            input: Input {
                room: crate::board::app::workspace::tests::ROOM.into(),
                resumes: Default::default(),
                names: Default::default(),
            },
            settings: TokenRate {
                enabled: usage_enabled,
                ..Default::default()
            },
            history: None,
        },
    );
    app
}

fn standings() -> Vec<Standing> {
    vec![Standing {
        provider: Provider::named("codex"),
        latest: Some(Reading {
            observed_at_ms: NOW - 60_000,
            weekly: Window {
                left: 78.0,
                resets_at_ms: NOW + 58 * HOUR,
            },
            short: None,
        }),
        burn: Some(0.4),
        trend: None,
    }]
}

fn rows(app: &App, width: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
    crate::status::with_now_ms(NOW, || {
        terminal
            .draw(|frame| crate::board::view::render(frame, app))
            .unwrap();
    });
    terminal
        .backend()
        .buffer()
        .content
        .chunks(usize::from(width))
        .map(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>())
        .map(|row| row.trim_end().to_owned())
        .collect()
}

fn index_of(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("no row with {needle:?} in {rows:#?}"))
}

#[test]
fn the_row_holds_its_place_before_the_first_sample_and_moves_nothing_when_it_fills() {
    for width in [100, 160] {
        let mut app = app(true);
        let mut pending_readings = standings();
        pending_readings[0].latest = None;
        app.set_limits(pending_readings);
        let before = rows(&app, width);
        let pending = index_of(&before, "Codex");
        let body_before = index_of(&before, "worker");
        let hits_before = format!("{:?}", app.hits.borrow());
        assert!(app.set_limits(standings()));
        let after = rows(&app, width);
        assert_eq!(index_of(&after, "78%"), pending, "{width}");
        assert_eq!(index_of(&after, "worker"), body_before, "{width}");
        assert_eq!(format!("{:?}", app.hits.borrow()), hits_before);
    }
}

#[test]
fn provider_quota_shares_its_usage_row_and_the_body_follows() {
    let mut app = app(true);
    app.set_limits(standings());
    let rows = rows(&app, 100);
    let quota = &rows[index_of(&rows, "78%")];
    assert!(quota.contains("Codex") && quota.contains("7d"), "{quota}");
    assert!(rows.iter().all(|row| !row.contains("Subscriptions")));
    assert_eq!(index_of(&rows, "78%"), index_of(&rows, "Usage · last") + 2);
    crate::status::with_now_ms(NOW, || {
        assert_eq!(app.open_quota_details(), crate::board::app::Effect::None);
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let details: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            details.contains("7d: 78% remaining · reset 2d10h"),
            "{details}"
        );
        assert!(details.contains("Weekly pace: ~0.4%/h"), "{details}");
    });
    assert!(index_of(&rows, "worker") > index_of(&rows, "78%"));
}

#[test]
fn meters_remain_at_narrow_widths_but_sampling_off_hides_them() {
    let mut narrow = app(true);
    narrow.set_limits(standings());
    for width in [160, 100, 80, 60] {
        let shown = rows(&narrow, width);
        assert!(
            shown.iter().any(|row| row.contains("78%")),
            "{width}: {shown:#?}"
        );
    }
    let mut off = app(false);
    off.set_limits(standings());
    let shown = rows(&off, 160);
    assert!(
        shown
            .iter()
            .all(|row| !row.contains("Codex") && !row.contains("limits"))
    );
}

#[test]
fn a_board_without_provider_evidence_keeps_one_honest_empty_state() {
    let mut app = app(true);
    let waiting = rows(&app, 100);
    assert_eq!(
        waiting
            .iter()
            .filter(|row| row.contains("No token observations yet"))
            .count(),
        1
    );
    let status = index_of(&waiting, "No token observations yet");
    assert!(!waiting[status].contains("unknown"));
    app.set_limits(Vec::new());
    let settled = rows(&app, 100);
    assert!(
        settled
            .iter()
            .all(|row| !row.contains("limits") && !row.contains("Codex"))
    );
    assert_eq!(
        settled
            .iter()
            .filter(|row| row.contains("No token observations yet"))
            .count(),
        1
    );
    assert_eq!(index_of(&settled, "worker"), index_of(&waiting, "worker"));
}

#[test]
fn an_unchanged_sample_paints_nothing_and_a_changed_one_does() {
    let mut app = app(true);
    assert!(app.set_limits(standings()));
    assert!(!app.set_limits(standings()));
    let mut moved = standings();
    moved[0].burn = Some(0.5);
    assert!(app.set_limits(moved));
}

#[test]
fn help_explains_the_row_only_where_it_can_show() {
    let section = |app: &App| {
        crate::board::help::model(app)
            .sections
            .into_iter()
            .find(|section| section.title == "provider limits")
    };
    let help = section(&app(true)).expect("HOME with the usage meter on explains the row");
    let text = help
        .entries
        .iter()
        .map(|entry| format!("{} {}", entry.keys, entry.description))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("remaining share inside each reported window bar"),
        "{text}"
    );
    assert!(text.contains("older than 2 hours"), "{text}");
    assert!(section(&app(false)).is_none());
    let mut hidden = app(true);
    hidden.shell.tab = crate::board::shell::OpsTab::Agents;
    assert!(section(&hidden).is_none());
    hidden.shell.tab = crate::board::shell::OpsTab::Digest;
    assert!(section(&hidden).is_some());
    hidden.shell.tab = crate::board::shell::OpsTab::Overview;
    hidden.set_limits(Vec::new());
    assert!(section(&hidden).is_none());
}
