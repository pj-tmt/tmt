//! The strip painter as it was before #1673: every span becomes a two-node
//! template, goes through Taffy geometry and `paint::paint`. It is the oracle
//! the direct painter must equal cell for cell; nothing else uses it.
use crate::{
    binding::{self, Schema, Schemas, Scopes, Sources},
    geometry, paint, parse,
};
use ratatui::{buffer::Buffer, layout::Rect, text::Line};
use std::{collections::BTreeMap, sync::OnceLock};
use tmt_cli_style::{Depth, Theme};

const FILE: &str = "tmt-tui.strip.xml";
const MARKUP: &str =
    r#"<tmt-view version="1"><tmt-text bind="$.text" class="w-full h-full"/></tmt-view>"#;

struct Display;
impl Sources for Display {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("strips bind display text only".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("strips acquire no sources".into())
    }
}

fn template() -> &'static binding::Template<()> {
    static TEMPLATE: OnceLock<binding::Template<()>> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let schema = Schema::Object(BTreeMap::from([("text".into(), Schema::Scalar)]));
        let parsed = parse(FILE, MARKUP).expect("embedded strip markup");
        binding::compile(FILE, &parsed, &schema, &Display).expect("embedded strip schema")
    })
}

/// The painter before #1673, kept as the oracle: paint `line` from the left edge of `area`; text beyond its width is clipped.
pub(super) fn paint_left(
    buffer: &mut Buffer,
    area: Rect,
    line: Line<'_>,
    theme: &Theme,
    depth: Depth,
) {
    paint_from(buffer, area, area.x, line, theme, depth);
}

/// Paint `line` flush with the right edge of `area`; a line wider than the
/// area keeps its start, as `paint_left` does.
pub(super) fn paint_right(
    buffer: &mut Buffer,
    area: Rect,
    line: Line<'_>,
    theme: &Theme,
    depth: Depth,
) {
    let width = line.width().min(usize::from(area.width)) as u16;
    paint_from(buffer, area, area.right() - width, line, theme, depth);
}

fn paint_from(
    buffer: &mut Buffer,
    area: Rect,
    start: u16,
    line: Line<'_>,
    theme: &Theme,
    depth: Depth,
) {
    let mut x = start.max(area.x);
    for span in line.spans {
        let width = span
            .width()
            .min(usize::from(area.right().saturating_sub(x))) as u16;
        if width == 0 {
            continue;
        }
        // The fixed schema takes an object with a string, and the two-node tree
        // has no sources, repeats or conditions, so finite u16 geometry cannot
        // make it invalid. A future failure skips the span instead of panicking
        // inside a frame.
        let Ok(mut node) =
            template().materialize(FILE, &serde_json::json!({"text": span.content}), &Display)
        else {
            continue;
        };
        node.selected = true;
        let Ok(mut cells) = geometry::layout(&node, [width, area.height], crate::text::measure)
        else {
            continue;
        };
        for cell in &mut cells {
            for rect in [&mut cell.rect, &mut cell.content, &mut cell.clip] {
                rect.x += i32::from(x);
                rect.y += i32::from(area.y);
            }
        }
        paint::paint(&cells, buffer, theme, depth, |_| {
            line.style.patch(span.style)
        });
        x += width;
    }
}
