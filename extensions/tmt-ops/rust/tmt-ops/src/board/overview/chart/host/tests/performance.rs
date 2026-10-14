//! Same local P02 budgets with real interval data, explicit reporter population.
use super::*;
use crate::board::{
    app::workspace::tests::populated_board,
    timing::probe::{measure, report},
};
#[test]
#[ignore = "explicit release M08 populated-chart P02 evidence, not a CI/user SLO"]
fn populated_chart_frame_cost() {
    if cfg!(debug_assertions) {
        panic!("run this probe with --release");
    }
    crate::status::with_now_ms(fixtures::NOW, || {
        for agents in [150, 1_000, 10_000] {
            let mut app = populated_board(36, agents);
            fixtures::populate(&mut app);
            let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
            for _ in 0..10 {
                measure(&mut app, &mut terminal, |_, _| {});
            }
            assert_eq!(
                app.usage_chart
                    .borrow()
                    .observed
                    .as_ref()
                    .unwrap()
                    .buckets
                    .len(),
                12
            );
            assert!(
                app.usage_chart
                    .borrow()
                    .observed
                    .as_ref()
                    .unwrap()
                    .buckets
                    .iter()
                    .any(|bucket| bucket.tokens.is_some_and(|value| value > 0))
            );
            for action in [
                "chart-steady",
                "chart-source-replace",
                "chart-scroll",
                "chart-resize",
                "chart-controls",
            ] {
                let samples = (0..100)
                    .map(|index| {
                        measure(&mut app, &mut terminal, |app, terminal| match action {
                            "chart-source-replace" => app.usage_chart.get_mut().invalidate_data(),
                            "chart-scroll" => {
                                press(
                                    app,
                                    if index % 2 == 0 {
                                        KeyCode::Down
                                    } else {
                                        KeyCode::Up
                                    },
                                );
                            }
                            "chart-resize" => {
                                app.invalidate_frame_geometry();
                                let width = if index % 2 == 0 { 100 } else { 160 };
                                terminal.backend_mut().resize(width, 36);
                                terminal.resize(Rect::new(0, 0, width, 36)).unwrap();
                            }
                            "chart-controls" => {
                                let mut state = app.usage_chart.borrow_mut();
                                state
                                    .apply(Target::Each(if index % 2 == 0 { 15 } else { 5 }))
                                    .unwrap();
                            }
                            _ => {}
                        })
                    })
                    .collect();
                let p95 = report(agents, action, samples);
                let cap = if agents == 150 { 10_000.0 } else { 50_000.0 };
                assert!(p95 <= cap, "{agents}/{action} p95 {p95} us exceeds {cap}");
            }
            eprintln!(
                "M08 population agents={agents} squads=36 reporting_identities=1 source_buckets=120 interval_bars=12; non-reporting identities remain partial"
            );
        }
    });
}
