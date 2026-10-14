//! Explicit actual-renderer evidence for WP3's new read-only modal surfaces.
use crate::board::{
    app::{Effect, Request},
    cronboard::{self, CronRequest},
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

#[test]
#[ignore = "explicit WP3 detail and timer-review evidence export"]
fn capture_monitor_modal_packet() {
    let output = std::env::var("TMT_MONITOR_MODAL_CAPTURE").expect("task-owned evidence path");
    let mut frames = Vec::new();
    for scene in ["details", "timer-pending", "timer-ready", "timer-failed"] {
        for width in [160, 140, 100, 80, 60] {
            for height in [36, 8] {
                for (base, depth) in [
                    ("tmt", tmt_cli_style::Depth::TrueColor),
                    ("tmt-light", tmt_cli_style::Depth::TrueColor),
                    ("terminal", tmt_cli_style::Depth::Ansi16),
                    ("tmt", tmt_cli_style::Depth::None),
                ] {
                    let look = crate::look::Look {
                        theme: tmt_cli_style::Theme::new(
                            tmt_cli_style::theme::Base::parse(base).unwrap(),
                        ),
                        depth,
                    };
                    let mut app =
                        crate::board::app::workspace::tests::capture_case("overview", look);
                    if scene == "details" {
                        assert_eq!(app.overview_details(), Effect::None);
                        assert!(app.monitor_details.is_some());
                    } else {
                        app.cron.replace(Ok(cronboard::test_cron(
                            vec![],
                            tmt_ops::cron::ClockStatus::NoClock,
                        )));
                        let request = CronRequest::Add {
                            actor: app.cron.cron.as_ref().unwrap().actor.clone().unwrap(),
                            squad: "product".into(),
                            room_id: "room".into(),
                            owner: "owner".into(),
                            message: "Review tests; no tokens or dollar estimate".into(),
                            schedule: "every 30m".into(),
                            zone: "UTC".into(),
                        };
                        let Effect::Act(Request::CronPreview(task)) =
                            app.cron_review_request(request)
                        else {
                            panic!("the existing worker lane owns preview acquisition")
                        };
                        if scene == "timer-ready" {
                            let (task, result) = cronboard::preview::run(*task, 0);
                            app.cron_review_ready(task, result);
                        } else if scene == "timer-failed" {
                            app.cron_review_ready(
                                *task,
                                Err("Schedule observation unavailable".into()),
                            );
                        }
                    }
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal
                        .draw(|frame| crate::board::view::render(frame, &app))
                        .unwrap();
                    let buffer = terminal.backend().buffer();
                    let cells = buffer.content.iter().map(|cell| json!({
                        "symbol":cell.symbol(), "fg":format!("{:?}",cell.fg),
                        "bg":format!("{:?}",cell.bg), "modifier":format!("{:?}",cell.modifier),
                    })).collect::<Vec<_>>();
                    let lines = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                        .collect::<Vec<_>>();
                    assert!(lines.iter().all(|line| !line.chars().any(char::is_control)));
                    frames.push(
                        json!({"scene":scene,"width":width,"height":height,"base":base,
                        "depth":format!("{depth:?}"),"cells":cells,"lines":lines}),
                    );
                }
            }
        }
    }
    assert_eq!(frames.len(), 160);
    std::fs::write(output, serde_json::to_vec(&frames).unwrap()).unwrap();
}
