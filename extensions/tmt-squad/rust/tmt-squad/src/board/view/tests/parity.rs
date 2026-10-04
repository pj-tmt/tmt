//! Frozen output from #1087 selected-notebook implementation f8c73805; production row adoption is gated by this capture.
//! Existing CJK fixture strings intentionally exercise terminal width.
use super::*;
use crate::config::{Config, Layout};

fn capture(app: &mut App, width: u16) -> Value {
    app.set_body_width(width);
    let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    let mut styles = Vec::new();
    let mut runs: Vec<(usize, String, usize)> = Vec::new();
    for cell in &terminal.backend().buffer().content {
        // Intern all non-symbol cell state; runs retain wide-glyph continuation cells.
        let mut metadata = cell.clone();
        metadata.set_symbol("");
        let metadata = format!("{metadata:?}");
        let style = styles
            .iter()
            .position(|v| v == &metadata)
            .unwrap_or_else(|| {
                styles.push(metadata);
                styles.len() - 1
            });
        let symbol = cell.symbol();
        if let Some(last) = runs
            .last_mut()
            .filter(|last| last.1 == symbol && last.2 == style)
        {
            last.0 += 1;
        } else {
            runs.push((1, symbol.to_owned(), style));
        }
    }
    json!({"width": width, "styles": styles, "cells": runs,
        "hits": format!("{:?}", app.hits.borrow()),
        "tabs": format!("{:?}", app.tab_hits.borrow()),
        "starts": *app.row_starts.borrow()})
}

fn baseline() -> Value {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/markup-parity.toml");
    let config = Config::read(path).unwrap();
    let mut fixtures = Vec::new();
    for layout in [Layout::Crew, Layout::PrQueue, Layout::Minimal, Layout::Team] {
        let mut app = preset_board();
        // The baseline's document: a lead that is only a name.
        app.view.as_mut().unwrap().document["squad"]["lead"] = json!({"name": "sol"});
        assert_eq!(config.layout(layout.as_str()).unwrap(), layout);
        let view = app.view.as_mut().unwrap();
        view.rows = config.rows(layout.as_str()).unwrap();
        view.board = config.board(layout.as_str()).unwrap();
        view.bindings = config.bindings(true, &view.board.panes).unwrap();
        view.document["squad"]["layout"] = json!(layout.as_str());
        let projected = view.rows.value();
        view.document["columns"] = projected["columns"].clone();
        view.document["lines"] = projected["lines"].clone();
        if let Some(hidden) = projected.get("hidden_columns") {
            view.document["hidden_columns"] = hidden.clone();
        }
        let document = view.document.clone();
        let frames: Vec<_> = [120, 80, 120]
            .into_iter()
            .map(|width| capture(&mut app, width))
            .collect();
        assert_eq!(
            frames[0], frames[2],
            "resize must restore full cells and hit identities"
        );
        let folded_frames: Vec<_> = if app.bindings().contains_key("d") {
            app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
            let folded: Vec<_> = [120, 80, 120]
                .into_iter()
                .map(|width| capture(&mut app, width))
                .collect();
            assert_eq!(folded[0], folded[2], "manual fold survives resize");
            app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
            assert_eq!(
                frames[0],
                capture(&mut app, 120),
                "unfold restores full cells, styles and hit identities"
            );
            folded
        } else {
            Vec::new()
        };
        fixtures.push(
            json!({"layout": layout.as_str(), "frames": frames, "folded_frames": folded_frames,
            "ls_text": crate::status::text(&document, tmt_cli_style::Terminal::PLAIN),
            "ls_json": serde_json::to_string(&document).unwrap()}),
        );
    }
    json!({"source": "f8c73805d64538647f4fada66910cd652c225394", "fixtures": fixtures})
}

#[test]
fn markup_slices_preserve_frozen_board_and_list_bytes() {
    let actual = serde_json::to_string(&baseline()).unwrap();
    assert_eq!(actual, include_str!("parity.json").trim_end());
}

/// Regenerate only after explicit review of a behavior change; normal tests never write fixtures.
#[test]
#[ignore = "explicit fixture regeneration; see DEVELOPMENT"]
fn regenerate_markup_parity_fixture() {
    std::fs::write(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/board/view/tests/parity.json"
        ),
        serde_json::to_string(&baseline()).unwrap(),
    )
    .unwrap();
}

/// Decode interned styles and runs before requesting baseline-regeneration approval.
#[test]
#[ignore = "read-only parity inspection; see the tmt-tui development reference"]
fn inspect_markup_parity_diff() {
    fn cells(frame: &Value) -> Vec<(String, String)> {
        frame["cells"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|run| {
                let cell = (
                    run[1].as_str().unwrap().to_owned(),
                    frame["styles"][run[2].as_u64().unwrap() as usize]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                );
                std::iter::repeat_n(cell, run[0].as_u64().unwrap() as usize)
            })
            .collect()
    }
    let expected: Value = serde_json::from_str(include_str!("parity.json")).unwrap();
    let actual = baseline();
    assert_eq!(expected["source"], actual["source"]);
    assert_eq!(
        expected["fixtures"].as_array().unwrap().len(),
        actual["fixtures"].as_array().unwrap().len()
    );
    for (old, new) in expected["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .zip(actual["fixtures"].as_array().unwrap())
    {
        let layout = old["layout"].as_str().unwrap();
        assert_eq!(old["layout"], new["layout"]);
        for key in ["ls_text", "ls_json"] {
            assert_eq!(
                old[key], new[key],
                "{layout}: {key} must stay byte-identical"
            );
        }
        println!("{layout}: list text/JSON byte-identical");
        for kind in ["frames", "folded_frames"] {
            let before = old[kind].as_array().unwrap();
            let after = new[kind].as_array().unwrap();
            assert_eq!(before.len(), after.len());
            for (index, (old, new)) in before.iter().zip(after).enumerate() {
                assert_eq!(old["width"], new["width"]);
                let width = old["width"].as_u64().unwrap() as usize;
                let before = cells(old);
                let after = cells(new);
                assert_eq!(before.len(), after.len());
                let mut symbols = 0;
                let mut styles = 0;
                println!("{layout}/{kind}/{index}: {width}x30");
                for (offset, (old, new)) in before.iter().zip(&after).enumerate() {
                    if old == new {
                        continue;
                    }
                    symbols += usize::from(old.0 != new.0);
                    styles += usize::from(old.1 != new.1);
                    println!(
                        "  ({}, {}) symbol {:?} -> {:?}",
                        offset % width,
                        offset / width,
                        old.0,
                        new.0
                    );
                    if old.1 != new.1 {
                        println!("    style {} -> {}", old.1, new.1);
                    }
                }
                for key in ["hits", "tabs", "starts"] {
                    if old[key] != new[key] {
                        println!("  {key}: {} -> {}", old[key], new[key]);
                    } else {
                        println!("  {key}: unchanged");
                    }
                }
                println!("  changed symbols: {symbols}; changed styles: {styles}");
            }
        }
    }
}
