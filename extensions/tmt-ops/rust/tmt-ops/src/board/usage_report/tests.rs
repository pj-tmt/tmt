//! The held report, its rows, the reader's pacing and the panel inside the Digest
//! tab. The supplier's frozen vector is the document; nothing here reads Core.

use super::{
    Delivery, Reader, Report, Since, State, Timing, View,
    panel::{self, Row},
};
use crate::{
    board::app::{App, Choice, workspace::tests::board},
    core::Core,
    test_support::write_ready_executable,
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

fn vector() -> Value {
    let all: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tmt-digest/contracts/vectors/usage-report-v1.json"
        ))
        .expect("the supplier's frozen usage report vector"),
    )
    .unwrap();
    all["cases"][0]["expected"].clone()
}

fn report() -> Report {
    Report::parse(&vector()).unwrap()
}

fn utc() -> jiff::tz::TimeZone {
    jiff::tz::TimeZone::UTC
}

fn text(row: &Row) -> String {
    match row {
        Row::Rule(text) => text.clone(),
        Row::Chips(since) => format!("[chips {}]", since.text()),
        Row::Line(pieces) => pieces.iter().map(|(text, _)| text.as_str()).collect(),
    }
}

fn texts(report: &Report, width: u16) -> Vec<String> {
    panel::rows(&View::Report(report), report.since, width, &utc())
        .iter()
        .map(text)
        .collect()
}

/// The frozen vector with `buckets` series points per seat, as a real window has.
fn with_buckets(buckets: usize) -> Report {
    let mut report = report();
    for (index, seat) in report.seats.iter_mut().enumerate() {
        seat.series = (0..buckets)
            .map(|bucket| ((bucket + index) % 7) as u64 * 1000)
            .collect();
    }
    report
}

#[test]
fn the_frozen_vector_parses_into_the_typed_report() {
    let report = report();
    assert_eq!(report.since, Since::Day);
    assert_eq!(
        report
            .lines
            .iter()
            .map(|l| l.name.as_str())
            .collect::<Vec<_>>(),
        ["claude", "gpt", "unattributed"]
    );
    assert_eq!(report.seats.len(), 4);
    assert_eq!(report.seats[2].lines, ["unattributed"]);
    assert_eq!(report.unavailable.len(), 2);
    assert_eq!(report.limits.len(), 3);
    assert!(report.limits[0].stale && report.limits[0].elapsed_percent.is_none());
    assert_eq!(report.limits_unavailable[0].reason, "no_source");
    let exchange = report.exchange.as_ref().unwrap();
    assert_eq!(exchange.senders[0].count, 7);
    assert!(report.warnings.is_empty());
}

#[test]
fn malformed_documents_are_refused_and_additive_fields_ignored() {
    let mut extra = vector();
    extra["future"] = json!({"anything": true});
    extra["usage"]["seats"][0]["future"] = json!(1);
    assert!(Report::parse(&extra).is_ok());
    let broken = |edit: &dyn Fn(&mut Value)| {
        let mut document = vector();
        edit(&mut document);
        Report::parse(&document).is_err()
    };
    assert!(broken(&|d| d["version"] = json!(2)));
    assert!(broken(&|d| d["since"] = json!("1h")));
    assert!(broken(
        &|d| d["usage"]["lines"][0]["totals"]["input"] = json!(-1)
    ));
    assert!(broken(
        &|d| d["usage"]["lines"][0]["totals"]["input"] = json!(9_007_199_254_740_992u64)
    ));
    assert!(broken(&|d| d["usage"]["seats"][0]["series"] = json!("none")));
    assert!(broken(
        &|d| d["usage"]["seats"][0]["coverage"]["startsAtMs"] = json!("noon")
    ));
    assert!(broken(&|d| d["limits"][1]["windowMinutes"] = json!(0)));
    assert!(broken(&|d| d["limits"][1]["usedPercent"] = json!("40")));
    assert!(broken(
        &|d| d["usage"]["unavailable"][0]["retired"] = json!("no")
    ));
    assert!(broken(
        &|d| d["exchange"]["bySender"][0]["count"] = json!(null)
    ));
    assert!(broken(&|d| {
        d.as_object_mut().unwrap().remove("warnings");
    }));
    // The supplier may report no exchange at all.
    let mut flow_null = vector();
    flow_null["exchange"] = Value::Null;
    assert_eq!(Report::parse(&flow_null).unwrap().exchange, None);
}

const GOLDEN: [&str; 19] = [
    "[chips 24h]",
    "                               IN      OUT   CACHED    TOTAL  SEATS",
    " claude               ▆█     1.1k      210      600     1.3k  2 · 1 partial",
    " gpt                  ▆█     5.0k      800     3.0k     5.8k  1",
    " unattributed         ··      100       20        0      120  1 · 1 partial",
    " account · 5h      90% used · window reset, not observed since",
    " gpt · 5h          40% used · 80% of window gone · 40 pts under pace",
    " gpt · 7d          60% used · 86% of window gone · 26 pts under pace",
    " claude · no limit source yet",
    "                               IN      OUT   CACHED    TOTAL  COVERAGE",
    " codex-a              ▆█     5.0k      800     3.0k     5.8k",
    " claude-seat          ▆█     1.0k      200      600     1.2k",
    " codex-b              ··      100       20        0      120  partial · since 01-14 09:00",
    " retired-seat retired ··       50       10        0       60  partial · 1h of 24h",
    " idle-seat · no usage recorded in this window",
    " 10000000 · not found",
    " Top senders       claude-seat 7 · codex-a 3 · unknown 1",
    " Top recipients    claude-seat 4 · 10000000 4 · codex-a 3",
    "",
];

#[test]
fn the_rows_use_the_agreed_wording_at_both_capture_widths() {
    for width in [103, 148] {
        let rows = texts(&report(), width);
        let rule = &rows[0];
        assert!(rule.starts_with("── Usage report ─"), "{rule}");
        assert!(
            rule.ends_with(" 24h · as of 08:01 · newest 5m not included ──"),
            "{rule}"
        );
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(rule.as_str()),
            width as usize
        );
        assert_eq!(rows[1..], GOLDEN[..GOLDEN.len() - 1]);
    }
}

fn limit_text(pace: f64, elapsed: f64) -> String {
    let mut report = report();
    report.limits.truncate(2);
    report.limits.remove(0);
    report.limits[0].pace_delta_points = Some(pace);
    report.limits[0].elapsed_percent = Some(elapsed);
    texts(&report, 148)
        .into_iter()
        .find(|row| row.trim_start().starts_with("gpt · 5h"))
        .unwrap()
}

#[test]
fn pace_wording_changes_exactly_at_half_a_point() {
    assert!(limit_text(-0.6, 50.0).ends_with("1 pts under pace"));
    assert!(limit_text(-0.5, 50.0).ends_with("on pace"));
    assert!(limit_text(0.0, 50.0).ends_with("on pace"));
    assert!(limit_text(0.5, 50.0).ends_with("on pace"));
    assert!(limit_text(0.6, 50.0).ends_with("1 pts over pace"));
    assert!(limit_text(21.4, 61.0).ends_with("61% of window gone · 21 pts over pace"));
}

#[test]
fn a_line_spark_sums_only_the_seats_wholly_inside_that_line() {
    let mut report = with_buckets(4);
    // Seats spanning both lines feed neither spark.
    for seat in [0, 3] {
        report.seats[seat].lines = vec!["claude".into(), "gpt".into()];
    }
    let rows = texts(&report, 148);
    let claude = rows
        .iter()
        .find(|row| row.trim_start().starts_with("claude  "))
        .unwrap();
    assert!(
        claude.contains("····"),
        "no seat is wholly claude: {claude}"
    );
    let gpt = rows
        .iter()
        .find(|row| row.trim_start().starts_with("gpt  "))
        .unwrap();
    assert!(!gpt.contains('·'), "{gpt}");
    // A series of another length never lines up, so it is unknown, not guessed.
    report.seats[1].series.pop();
    let rows = texts(&report, 148);
    assert!(
        rows.iter()
            .find(|row| row.trim_start().starts_with("gpt  "))
            .unwrap()
            .contains("····")
    );
}

#[test]
fn a_live_window_comes_before_its_stale_namesake_and_every_row_has_the_gutter() {
    let mut report = report();
    // The stale 5h window belongs to the same line as the live one.
    report.limits[0].seats = report.limits[1].seats.clone();
    let rows = texts(&report, 148);
    let at = |needle: &str| rows.iter().position(|row| row.contains(needle)).unwrap();
    assert!(at("gpt · 5h          40% used") < at("gpt · 5h          90% used"));
    assert!(at("90% used") < at("gpt · 7d"));
    for row in panel::rows(&View::Report(&report), report.since, 148, &utc()) {
        match row {
            Row::Line(_) => assert!(text(&row).starts_with(' '), "{}", text(&row)),
            Row::Rule(rule) => assert!(rule.starts_with('─')),
            Row::Chips(_) => {}
        }
    }
}

#[test]
fn only_the_top_eight_seats_show_and_the_rest_point_to_the_export() {
    let mut report = report();
    let template = report.seats[0].clone();
    for index in 0..10 {
        let mut seat = template.clone();
        seat.who.id = Some(format!("20000000-0000-4000-8000-{index:012}"));
        seat.who.name = Some(format!("extra-{index}"));
        seat.input = 10_000 + index;
        report.seats.push(seat);
    }
    let rows = texts(&report, 148);
    let seats = rows
        .iter()
        .filter(|row| row.trim_start().starts_with("extra-"))
        .count();
    assert_eq!(seats, 8);
    assert!(rows.contains(&" +6 more · tmt digest usage report --csv usage".to_string()));
    // Largest first.
    assert!(
        rows.iter()
            .position(|row| row.trim_start().starts_with("extra-9"))
            < rows
                .iter()
                .position(|row| row.trim_start().starts_with("extra-8"))
    );
}

#[test]
fn views_without_a_report_say_so_and_unavailable_reads_are_not_zero() {
    let rows = |view: View<'_>| -> Vec<String> {
        panel::rows(&view, Since::Week, 100, &utc())
            .iter()
            .map(text)
            .collect()
    };
    let loading = rows(View::Loading);
    assert_eq!(loading[1], "[chips 7d]");
    assert_eq!(loading[2], " Loading usage report…");
    assert!(
        rows(View::Failed("Core is down"))
            .contains(&" Usage report unavailable · Core is down".to_string())
    );
    let mut report = report();
    report.unavailable[1].reason = "read_failed".into();
    report.exchange = None;
    report.warnings = (0..5)
        .map(|n| super::model::Warning {
            section: "usage".into(),
            message: format!("m{n}"),
        })
        .collect();
    let shown = texts(&report, 148);
    assert!(shown.contains(&" ✗ 10000000 · could not be read".to_string()));
    assert!(shown.contains(&" Exchange flow unavailable for this window.".to_string()));
    assert!(shown.contains(&" warning · usage · m2".to_string()));
    assert!(shown.contains(&" +2 more warnings".to_string()));
    let mut empty = report.clone();
    empty.lines.clear();
    empty.seats.clear();
    empty.unavailable.clear();
    assert!(texts(&empty, 100).contains(&" No usage recorded in this window.".to_string()));
}

#[test]
fn no_row_is_wider_than_the_panel_at_any_width() {
    for report in [report(), with_buckets(24), with_buckets(30)] {
        for width in 0..=170u16 {
            for row in panel::rows(&View::Report(&report), report.since, width, &utc()) {
                let shown = text(&row);
                if matches!(row, Row::Chips(_)) {
                    continue;
                }
                assert!(
                    unicode_width::UnicodeWidthStr::width(shown.as_str()) <= usize::from(width),
                    "width {width}: {shown:?}"
                );
            }
        }
    }
}

#[test]
fn coverage_text_outranks_the_spark_when_both_do_not_fit() {
    let report = with_buckets(30);
    let rows = texts(&report, 103);
    let partial = rows
        .iter()
        .find(|row| row.trim_start().starts_with("codex-b"))
        .unwrap();
    assert!(
        partial.ends_with("partial · since 01-14 09:00"),
        "{partial}"
    );
    assert!(
        !partial.contains('▁') && !partial.contains('█'),
        "{partial}"
    );
    let wide = texts(&report, 148);
    let partial = wide
        .iter()
        .find(|row| row.trim_start().starts_with("codex-b"))
        .unwrap();
    assert!(
        partial.ends_with("partial · since 01-14 09:00"),
        "{partial}"
    );
    assert!(wide.iter().any(|row| row.contains('█')));
}

#[test]
fn partial_coverage_names_where_it_began_when_that_is_inside_the_window() {
    let partial = |report: &Report, seat: &str| {
        texts(report, 148)
            .into_iter()
            .find(|row| row.trim_start().starts_with(seat))
            .unwrap()
    };
    let report = report();
    // codex-b was first recorded an hour into the window, on the day before the report.
    let start = report.seats[2].starts_at_ms.unwrap();
    assert_eq!(
        Some(start),
        report.window_from_ms.map(|from| from + 3_600_000)
    );
    assert!(partial(&report, "codex-b").ends_with("partial · since 01-14 09:00"));
    // The same start on the report's own day is a time of day.
    let mut today = report.clone();
    today.observed_at_ms = start + 3_600_000;
    assert!(partial(&today, "codex-b").ends_with("partial · since 09:00"));
    // Evidence that starts with the window is partial by its gaps: covered of window.
    assert!(partial(&report, "retired-seat").ends_with("partial · 1h of 24h"));
    // A supplier that does not say where evidence began leaves the covered time.
    let mut silent = report.clone();
    silent.seats[2].starts_at_ms = None;
    assert!(partial(&silent, "codex-b").ends_with("partial · 12h of 24h"));
    let mut unread = report.clone();
    unread.window_from_ms = None;
    assert!(partial(&unread, "codex-b").ends_with("partial · 12h of 24h"));
    // An older document without the field still parses.
    let mut old = vector();
    for seat in old["usage"]["seats"].as_array_mut().unwrap() {
        seat["coverage"]
            .as_object_mut()
            .unwrap()
            .remove("startsAtMs");
    }
    let old = Report::parse(&old).unwrap();
    assert!(old.seats.iter().all(|seat| seat.starts_at_ms.is_none()));
}

#[test]
fn durations_read_as_hours_then_days() {
    assert_eq!(panel::duration(0), "0m");
    assert_eq!(panel::duration(90_000), "1m");
    assert_eq!(panel::duration(3_600_000), "1h");
    assert_eq!(panel::duration(18 * 3_600_000), "18h");
    assert_eq!(panel::duration(24 * 3_600_000), "24h");
    assert_eq!(panel::duration(48 * 3_600_000), "48h");
    assert_eq!(panel::duration(49 * 3_600_000), "2d 1h");
    assert_eq!(panel::duration(7 * 86_400_000), "7d");
}

fn delivery(since: Since, result: Result<Report, String>) -> Delivery {
    Delivery { since, result }
}

#[test]
fn a_failed_read_keeps_the_last_good_report_and_another_window_waits() {
    let mut state = State::default();
    assert!(matches!(state.view(), View::Loading));
    state.apply(delivery(Since::Day, Ok(report())));
    assert!(matches!(state.view(), View::Report(_)));
    state.apply(delivery(Since::Day, Err("down".into())));
    assert!(
        matches!(state.view(), View::Report(_)),
        "the old report stands"
    );
    state.since = Since::Week;
    assert!(
        matches!(state.view(), View::Loading),
        "another window is not the held one"
    );
    state.apply(delivery(Since::Week, Err("down".into())));
    assert!(matches!(state.view(), View::Failed("down")));
    state.since = Since::Day;
    assert!(matches!(state.view(), View::Report(_)));
}

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "ops-usage-report-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    /// A stand-in for `tmt` that logs each call and prints `report.json`, or fails.
    fn core(&self) -> Core {
        let script = self.0.join("tmt");
        write_ready_executable(
            &script,
            &format!(
                "#!/bin/sh\necho \"$*\" >> '{dir}/calls'\n\
                 if [ -f '{dir}/fail' ]; then echo '{{\"error\":{{\"code\":\"X\",\"message\":\"supplier down\"}}}}'; exit 1; fi\n\
                 cat '{dir}/report.json'\n",
                dir = self.0.display()
            ),
        );
        Core::at(script)
    }
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.0.join("calls"))
            .map(|calls| calls.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn quick() -> Timing {
    Timing {
        every: Duration::from_millis(30),
        deadline: Duration::from_secs(10),
        slowest: Duration::from_millis(120),
    }
}

fn next(received: &mpsc::Receiver<Delivery>) -> Delivery {
    received
        .recv_timeout(Duration::from_secs(20))
        .expect("a delivery")
}

#[test]
fn the_reader_reads_only_while_wanted_and_for_the_wanted_window() {
    let scratch = Scratch::new();
    std::fs::write(scratch.0.join("report.json"), vector().to_string()).unwrap();
    let (sender, received) = mpsc::channel();
    let reader = Reader::spawn(&scratch.core(), quick(), move |d| sender.send(d).is_ok());
    let wants = reader.wants();
    // Nothing is read before the panel is wanted.
    std::thread::sleep(Duration::from_millis(150));
    assert!(scratch.calls().is_empty());
    wants.set(Some(Since::Day));
    let first = next(&received);
    assert_eq!((first.since, first.result.is_ok()), (Since::Day, true));
    assert_eq!(scratch.calls()[0], "digest usage report --since 24h --json");
    // It repeats while wanted, and a new window reads at once for that window.
    let again = next(&received);
    assert_eq!(again.since, Since::Day);
    wants.set(Some(Since::Month));
    loop {
        let delivery = next(&received);
        if delivery.since == Since::Month {
            break;
        }
    }
    assert!(
        scratch
            .calls()
            .iter()
            .any(|call| call == "digest usage report --since 30d --json")
    );
    // Leaving the panel stops the reads.
    wants.set(None);
    std::thread::sleep(Duration::from_millis(150));
    while received.try_recv().is_ok() {}
    let settled = scratch.calls().len();
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(
        scratch.calls().len(),
        settled,
        "no read while the panel is off screen"
    );
    drop(reader);
}

#[test]
fn a_failing_supplier_is_delivered_as_a_message_and_a_malformed_document_is_refused() {
    let scratch = Scratch::new();
    std::fs::write(scratch.0.join("fail"), "").unwrap();
    let (sender, received) = mpsc::channel();
    let reader = Reader::spawn(&scratch.core(), quick(), move |d| sender.send(d).is_ok());
    reader.wants().set(Some(Since::Week));
    let failed = next(&received);
    assert_eq!(failed.since, Since::Week);
    assert_eq!(failed.result.unwrap_err(), "supplier down");
    std::fs::remove_file(scratch.0.join("fail")).unwrap();
    std::fs::write(scratch.0.join("report.json"), "{\"version\":2}").unwrap();
    loop {
        let delivery = next(&received);
        if let Err(message) = delivery.result
            && message != "supplier down"
        {
            assert!(message.contains("not version 1"), "{message}");
            break;
        }
    }
    drop(reader);
}

fn frame(app: &mut App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

fn digest_board(held: bool) -> App {
    let mut app = board();
    app.shell.tab = crate::board::shell::OpsTab::Digest;
    if held {
        app.usage_report.apply(delivery(Since::Day, Ok(report())));
    }
    app
}

#[test]
fn the_panel_is_wanted_only_on_the_digest_tab_while_it_is_shown() {
    use crate::config::{DigestPanel, DigestPanels, PanelSetting};
    let mut app = digest_board(false);
    assert_eq!(app.usage_report_want(), Some(Since::Day));
    app.usage_report.since = Since::Month;
    assert_eq!(app.usage_report_want(), Some(Since::Month));
    app.shell.tab = crate::board::shell::OpsTab::Agents;
    assert_eq!(app.usage_report_want(), None);
    app.shell.tab = crate::board::shell::OpsTab::Digest;
    let mut policy = DigestPanels::default();
    policy.set_shown(DigestPanel::Report, false).unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .digest_panels = Some(PanelSetting::configured(policy));
    assert_eq!(app.usage_report_want(), None);
}

#[test]
fn the_digest_tab_paints_the_panel_and_keeps_the_member_list() {
    let mut app = digest_board(true);
    for (width, height) in [(103, 51), (148, 51)] {
        let shown = frame(&mut app, width, height);
        let joined = shown.join("\n");
        assert!(joined.contains("── Usage report ─"), "{joined}");
        assert!(joined.contains("Top senders"), "{joined}");
        assert!(joined.contains("claude-seat"));
        // The member list still has its heading and the worker row.
        assert!(joined.contains("worker"), "{joined}");
        assert!(shown.len() == usize::from(height));
    }
    // Hidden: nothing of it paints.
    use crate::config::{DigestPanel, DigestPanels, PanelSetting};
    let mut policy = DigestPanels::default();
    policy.set_shown(DigestPanel::Report, false).unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .preferences
        .digest_panels = Some(PanelSetting::configured(policy));
    assert!(!frame(&mut app, 148, 51).join("\n").contains("Usage report"));
}

#[test]
fn a_loading_panel_paints_at_once_and_a_short_frame_never_hides_the_member_list() {
    let mut app = digest_board(false);
    let shown = frame(&mut app, 103, 51).join("\n");
    assert!(shown.contains("Loading usage report…"), "{shown}");
    app.usage_report.apply(delivery(Since::Day, Ok(report())));
    for height in [0u16, 1, 6, 10, 14, 20, 30] {
        for width in [0u16, 1, 10, 40, 80, 103, 148] {
            let shown = frame(&mut app, width, height);
            assert_eq!(shown.len(), usize::from(height));
        }
    }
    // A tall enough tab clips the report before the member list loses its floor.
    let short = frame(&mut app, 103, 24).join("\n");
    assert!(short.contains("worker"), "{short}");
    assert!(short.contains("more rows"), "{short}");
}

#[test]
fn a_chip_click_and_the_settings_menu_choose_the_window() {
    let mut app = digest_board(true);
    frame(&mut app, 148, 51);
    let hits = app.workspace_strip_hits.borrow().clone();
    let (rect, _) = hits
        .iter()
        .find(|(_, choice)| *choice == Choice::UsageReportSince(Since::Week))
        .expect("the 7d chip is a hit");
    assert!(
        hits.iter()
            .any(|(_, c)| *c == Choice::UsageReportSince(Since::Month))
    );
    assert!(rect.width >= 4 && rect.height == 1);
    app.choose(Choice::UsageReportSince(Since::Week));
    assert_eq!(app.usage_report.since, Since::Week);
    assert_eq!(app.usage_report_want(), Some(Since::Week));
    // Not yet read for that window: the panel waits, it does not show the old one.
    assert!(
        frame(&mut app, 148, 51)
            .join("\n")
            .contains("Loading usage report…")
    );
    // The menu offers the same three, the current one marked.
    app.workspace_settings();
    let labels: Vec<_> = app
        .menu
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .map(|entry| entry.label.clone())
        .filter(|label| label.starts_with("Usage report window"))
        .collect();
    assert_eq!(
        labels,
        [
            "Usage report window: 24h",
            "Usage report window: 7d (current)",
            "Usage report window: 30d"
        ]
    );
    // Other tabs do not offer it.
    app.shell.tab = crate::board::shell::OpsTab::Agents;
    app.workspace_settings();
    assert!(
        !app.menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .any(|e| e.label.starts_with("Usage report"))
    );
}

#[test]
fn paint_and_input_start_no_command() {
    let _no_commands = crate::runner::forbid_commands();
    let mut app = digest_board(true);
    frame(&mut app, 103, 51);
    app.choose(Choice::UsageReportSince(Since::Month));
    frame(&mut app, 103, 51);
    let started = Instant::now();
    frame(&mut app, 148, 51);
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// A busy fleet: two lines, ten seats, 24 hourly buckets, one account window.
fn fleet() -> Report {
    let mut report = report();
    report.seats.clear();
    report.unavailable.clear();
    let names = [
        "tmt-ux-lead",
        "night-2",
        "core-ops",
        "tmt-lead",
        "colab-1",
        "remote-4",
        "infra-1",
        "ux-1",
        "tmt-core-pm",
        "mkt-1",
    ];
    for (index, name) in names.iter().enumerate() {
        let scale = (names.len() - index) as u64;
        let line = if index % 3 == 0 { "gpt" } else { "claude" };
        let series: Vec<u64> = (0..24)
            .map(|hour| scale * 90_000 * (1 + (hour * 7 + index as u64) % 9))
            .collect();
        let total: u64 = series.iter().sum();
        report.seats.push(super::model::Seat {
            who: super::model::Who {
                id: Some(format!("30000000-0000-4000-8000-{index:012}")),
                name: Some((*name).into()),
                retired: false,
            },
            input: total * 9 / 10,
            output: total / 10,
            cached: total * 7 / 10,
            lines: vec![line.into()],
            series,
            covered_ms: 86_400_000,
            window_ms: 86_400_000,
            starts_at_ms: None,
            complete: true,
        });
    }
    for line in &mut report.lines {
        let seats: Vec<_> = report
            .seats
            .iter()
            .filter(|s| s.lines[0] == line.name)
            .collect();
        line.seats = seats.len() as u64;
        line.partial_seats = 0;
        line.input = seats.iter().map(|s| s.input).sum();
        line.output = seats.iter().map(|s| s.output).sum();
        line.cached = seats.iter().map(|s| s.cached).sum();
    }
    report.lines.retain(|line| line.seats > 0);
    let tally = |name: &str, count| super::model::Tally {
        who: super::model::Who {
            id: None,
            name: Some(name.into()),
            retired: false,
        },
        count,
    };
    report.exchange = Some(super::model::Traffic {
        senders: vec![
            tally("tmt-ux-lead", 42),
            tally("night-2", 31),
            tally("core-ops", 24),
            tally("tmt-lead", 18),
            tally("colab-1", 9),
        ],
        recipients: vec![
            tally("tmt-lead", 40),
            tally("core-ops", 33),
            tally("night-2", 21),
            tally("ux-1", 12),
            tally("remote-4", 7),
        ],
    });
    for limit in &mut report.limits {
        limit.seats = report
            .seats
            .iter()
            .filter(|s| s.lines[0] == "gpt")
            .filter_map(|s| s.who.id.clone())
            .collect();
    }
    report
}

/// The five states tmt-ux-lead asked to see, at both capture sizes, as plain
/// frames: `TMT_USAGE_REPORT_OUTPUT=<dir> cargo test -p tmt-ops capture_usage -- --ignored`.
#[test]
#[ignore = "writes the review captures to TMT_USAGE_REPORT_OUTPUT"]
fn capture_usage_report_packet() {
    let output = std::path::PathBuf::from(
        std::env::var_os("TMT_USAGE_REPORT_OUTPUT").expect("TMT_USAGE_REPORT_OUTPUT"),
    );
    std::fs::create_dir_all(&output).unwrap();
    let states: [(&str, Report); 5] = [
        ("populated", fleet()),
        ("partial", {
            let mut report = fleet();
            for seat in report.seats.iter_mut().step_by(2) {
                seat.complete = false;
                seat.covered_ms = 64_800_000;
            }
            for line in &mut report.lines {
                line.partial_seats = report
                    .seats
                    .iter()
                    .filter(|s| s.lines[0] == line.name && !s.complete)
                    .count() as u64;
            }
            report
        }),
        ("unavailable", {
            let mut report = fleet();
            report.unavailable = report::unavailable_rows();
            report
        }),
        ("stale", {
            let mut report = fleet();
            report.limits[0].stale = true;
            report.limits[0].elapsed_percent = None;
            report.limits[0].pace_delta_points = None;
            report
        }),
        ("flow-null", {
            let mut report = fleet();
            report.exchange = None;
            report.warnings = vec![super::model::Warning {
                section: "exchange".into(),
                message: "Core could not read the exchange flow".into(),
            }];
            report
        }),
    ];
    crate::status::with_now_ms(1_800_000_060_000, || {
        for (name, report) in states {
            for (width, height) in [(103u16, 51u16), (148, 51)] {
                let mut app = digest_board(false);
                app.usage_report
                    .apply(delivery(Since::Day, Ok(report.clone())));
                let shown = frame(&mut app, width, height).join("\n");
                std::fs::write(
                    output.join(format!("{name}-{width}x{height}.txt")),
                    shown + "\n",
                )
                .unwrap();
            }
        }
    });
}

mod report {
    use super::super::model::{Unavailable, Who};
    pub fn unavailable_rows() -> Vec<Unavailable> {
        let who = |name: Option<&str>, id: &str, retired| Who {
            id: Some(id.into()),
            name: name.map(str::to_owned),
            retired,
        };
        vec![
            Unavailable {
                who: who(
                    Some("ux-revamp"),
                    "40000000-0000-4000-8000-000000000001",
                    false,
                ),
                reason: "no_evidence".into(),
            },
            Unavailable {
                who: who(None, "40000000-0000-4000-8000-000000000002", true),
                reason: "not_found".into(),
            },
            Unavailable {
                who: who(
                    Some("meteor-3"),
                    "40000000-0000-4000-8000-000000000003",
                    false,
                ),
                reason: "read_failed".into(),
            },
        ]
    }
}
