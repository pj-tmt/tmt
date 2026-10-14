//! Shared opt-in test probes over the actual board frame and buffer diff.
use crate::board::app::App;
use ratatui::{Terminal, backend::TestBackend};
use std::time::{Duration, Instant};

pub(in crate::board) struct Sample {
    event_draw: Duration,
    changed_cells: usize,
}

pub(in crate::board) fn measure(
    app: &mut App,
    terminal: &mut Terminal<TestBackend>,
    event: impl FnOnce(&mut App, &mut Terminal<TestBackend>),
) -> Sample {
    let previous = terminal.backend().buffer().clone();
    let start = Instant::now();
    event(app, terminal);
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    let event_draw = start.elapsed();
    let current = terminal.backend().buffer();
    // Counting is outside the timed event/draw. Resize is a full clear/redraw;
    // this is changed buffer cells, not ANSI bytes or physical terminal output.
    let changed_cells = if current.area == previous.area {
        previous.diff(current).len()
    } else {
        current.content.len()
    };
    Sample {
        event_draw,
        changed_cells,
    }
}

pub(in crate::board) fn report(rows: usize, action: &str, samples: Vec<Sample>) -> f64 {
    if let Ok(directory) = std::env::var("TMT_OPS_FRAME_SAMPLES") {
        let directory = std::path::Path::new(&directory);
        std::fs::create_dir_all(directory).unwrap();
        let values = samples.iter().map(|sample| serde_json::json!({"eventDrawUs":sample.event_draw.as_secs_f64()*1_000_000.0,"changedCells":sample.changed_cells})).collect::<Vec<_>>();
        std::fs::write(
            directory.join(format!("{rows}-{action}.json")),
            serde_json::to_vec(&values).unwrap(),
        )
        .unwrap();
    }
    let mut times: Vec<_> = samples.iter().map(|sample| sample.event_draw).collect();
    let mut cells: Vec<_> = samples.iter().map(|sample| sample.changed_cells).collect();
    times.sort();
    cells.sort_unstable();
    let at = |quantile: f64| ((times.len() - 1) as f64 * quantile).round() as usize;
    eprintln!(
        "workspace agents={rows} squads=36 action={action} samples={} event_draw_us p50={:.1} p95={:.1} changed_cells p50={} p95={}",
        times.len(),
        times[at(0.5)].as_secs_f64() * 1_000_000.0,
        times[at(0.95)].as_secs_f64() * 1_000_000.0,
        cells[at(0.5)],
        cells[at(0.95)]
    );
    times[at(0.95)].as_secs_f64() * 1_000_000.0
}
