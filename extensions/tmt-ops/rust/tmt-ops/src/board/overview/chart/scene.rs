//! Template-backed usage chart. The host owns data, selection, cache and effects.
#[cfg(test)]
use super::model::Coverage;
use super::{
    model::Series,
    quantize::{self, Cell},
};
use ratatui::{
    buffer::Buffer,
    style::{Color, Modifier},
};
use serde_json::json;
use std::{collections::BTreeMap, sync::OnceLock};
use tmt_cli_style::{Depth, Role, Theme, grid::Align, theme::screen};
use tmt_tui::{
    binding::{Node, Schema, Schemas, Scopes, Sources, Template},
    geometry, paint, text,
};

const FILE: &str = "ops.overview.chart.xml";
const MARKUP: &str = r#"<tmt-view version="1" class="w-full">
<tmt-repeat each="$.rows" as="row"><tmt-row id-bind="row.id" class="w-full h-1">
<tmt-repeat each="row.pieces" as="piece"><tmt-text id-bind="piece.id" bind="piece.text" class="shrink-0"/></tmt-repeat>
</tmt-row></tmt-repeat></tmt-view>"#;

struct Data;
impl Sources for Data {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("chart uses bound values only".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("chart uses bound values only".into())
    }
}

fn template() -> &'static Template<()> {
    static TEMPLATE: OnceLock<Template<()>> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let piece = Schema::Object(BTreeMap::from([
            ("id".into(), Schema::StableId),
            ("text".into(), Schema::Scalar),
        ]));
        let row = Schema::Object(BTreeMap::from([
            ("id".into(), Schema::StableId),
            ("pieces".into(), Schema::Collection(Box::new(piece))),
        ]));
        let schema = Schema::Object(BTreeMap::from([(
            "rows".into(),
            Schema::Collection(Box::new(row)),
        )]));
        let parsed = tmt_tui::parse(FILE, MARKUP).expect("embedded chart markup");
        tmt_tui::binding::compile(FILE, &parsed, &schema, &Data).expect("embedded chart schema")
    })
}

fn time_ticks(minutes: u8, width: usize) -> String {
    let first = format!("−{minutes}m");
    let middle = if minutes.is_multiple_of(2) {
        format!("−{}m", minutes / 2)
    } else {
        format!("−{}.5m", minutes / 2)
    };
    let mut cells = vec![' '; width];
    let mut put = |at: usize, label: &str| {
        for (index, ch) in label.chars().enumerate() {
            if let Some(cell) = cells.get_mut(at + index) {
                *cell = ch;
            }
        }
    };
    let first_len = first.chars().count();
    put(0, &first);
    let last = width.saturating_sub(3);
    if last > first_len {
        put(last, "now");
    }
    let middle_len = middle.chars().count();
    let center = width.saturating_sub(middle_len) / 2;
    if center > first_len && center + middle_len < last {
        put(center, &middle);
    }
    cells.into_iter().collect()
}

pub struct Scene {
    node: Node,
    rows: u16,
}

impl Scene {
    /// Fixed work: at most 12 bars by 24 rows. Low-height hosts give a smaller
    /// height; no clock, history read, selection or automatic aggregation here.
    pub fn new(series: &Series, width: u16, height: u8) -> Result<Self, String> {
        Series::new(series.selection, series.buckets.clone()).map_err(str::to_owned)?;
        if !(1..=24).contains(&height) {
            return Err("Chart height must be 1–24 cells".into());
        }
        let scale = quantize::scale(series.buckets.iter().map(|bucket| bucket.tokens));
        let scale_label = scale.map_or_else(String::new, |value| {
            super::format::tokens(u128::from(value))
        });
        let middle_label = scale.map_or_else(String::new, |value| {
            super::format::tokens(u128::from(value / 2))
        });
        // Tick labels share the plot rows, so raw integer text never reserves an empty gutter.
        let axis_width = scale_label.len().max(middle_label.len()).max(1) + 1;
        let count = series.buckets.len();
        let bar_width = usize::from(width)
            .saturating_sub(axis_width + count - 1)
            .checked_div(count)
            .unwrap_or(1)
            .max(1);
        let bars: Vec<_> = series
            .buckets
            .iter()
            .map(|bucket| quantize::cells(bucket.tokens, scale.unwrap_or(0), height))
            .collect();
        let mut rows = Vec::new();
        for level in (0..usize::from(height)).rev() {
            let axis = if level == 0 && scale.is_some() {
                "0"
            } else if level + 1 == usize::from(height) {
                scale_label.as_str()
            } else if height >= 4 && level == usize::from(height) / 2 {
                middle_label.as_str()
            } else {
                ""
            };
            let mut pieces = vec![json!({"id":"axis", "text": format!("{axis:>axis_width$}")})];
            for (index, bar) in bars.iter().enumerate() {
                let (kind, text) = if series.buckets[index].tokens.is_none() {
                    if level < usize::from(height).div_ceil(3) {
                        ("unknown", "·".repeat(bar_width))
                    } else {
                        ("empty", " ".repeat(bar_width))
                    }
                } else {
                    match bar[level] {
                        Cell::Empty => ("empty", " ".repeat(bar_width)),
                        Cell::Full => ("full", " ".repeat(bar_width)),
                        Cell::LowerHalf => ("half", "▄".repeat(bar_width)),
                        Cell::Unknown => ("unknown", "·".repeat(bar_width)),
                    }
                };
                pieces.push(json!({"id":format!("{kind}-bar-{index}"), "text":text}));
                if index + 1 < count {
                    pieces.push(json!({"id":format!("gap-{index}"), "text":" "}));
                }
            }
            rows.push(json!({"id":format!("level-{level}"), "pieces":pieces}));
        }
        let ticks = time_ticks(
            series.selection.range_minutes,
            bar_width * count + count - 1,
        );
        rows.push(json!({"id":"time", "pieces":[{"id":"axis", "text":" ".repeat(axis_width)}, {"id":"time-ticks", "text":ticks}]}));
        let row_count = rows.len() as u16;
        let node = template()
            .materialize(FILE, &json!({"rows":rows}), &Data)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            node,
            rows: row_count,
        })
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Hits come from the same solved/clipped nodes as paint. Resizing requires
    /// the caller to prepare a new scene; it never reuses an old frame's map.
    pub fn paint<'a>(
        &'a self,
        buffer: &mut Buffer,
        theme: &Theme,
        depth: Depth,
    ) -> Result<Vec<paint::Hit<'a>>, String> {
        let area = buffer.area;
        let mut cells = geometry::layout(&self.node, [area.width, area.height], text::measure)?;
        for cell in &mut cells {
            for rect in [&mut cell.rect, &mut cell.content, &mut cell.clip] {
                rect.x += i32::from(area.x);
                rect.y += i32::from(area.y);
            }
        }
        let filled = screen::style(theme, Role::Accent, depth);
        let muted = screen::style(theme, Role::Muted, depth);
        let hits = paint::paint_with(&cells, buffer, |index, _, _| {
            let id = cells[index]
                .node
                .id
                .as_ref()
                .and_then(|id| id.last())
                .map(String::as_str)
                .unwrap_or("");
            let style = if id.starts_with("full-") {
                if depth == Depth::None {
                    ratatui::style::Style::default()
                } else if filled.fg.is_none_or(|color| color == Color::Reset) {
                    filled.add_modifier(Modifier::REVERSED)
                } else {
                    ratatui::style::Style::default().bg(filled.fg.unwrap_or_default())
                }
            } else if id.starts_with("half-") {
                filled
            } else if id.starts_with("unknown-") {
                screen::style(theme, Role::Dim, depth)
            } else {
                muted
            };
            (style, Align::Left)
        });
        if depth == Depth::None {
            for hit in &hits {
                if hit
                    .id
                    .as_ref()
                    .is_some_and(|id| id.last().is_some_and(|id| id.starts_with("full-")))
                {
                    for y in hit.rect.y..hit.rect.y + hit.rect.height as i32 {
                        for x in hit.rect.x..hit.rect.x + hit.rect.width as i32 {
                            buffer[(x as u16, y as u16)].set_symbol("█");
                        }
                    }
                }
            }
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{Bucket, Selection};
    use super::*;
    use ratatui::{layout::Rect, style::Color};
    use tmt_cli_style::theme::Base;

    fn series() -> Series {
        let selection = Selection::default();
        Series::new(
            selection,
            (0..12)
                .map(|index| Bucket {
                    from_ms: index * 300_000,
                    to_ms: (index + 1) * 300_000,
                    tokens: Some(match index {
                        0 => 0,
                        1 => 1,
                        2 => 50,
                        _ => 100,
                    }),
                    coverage: Coverage::Complete,
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn real_scale_ticks_leave_no_raw_number_gutter_and_partial_bars_have_no_mark() {
        let mut values = series();
        for bucket in &mut values.buckets {
            bucket.tokens = Some(23_058_051);
            bucket.coverage = Coverage::Partial {
                covered_ms: 200_000,
            };
        }
        let scene = Scene::new(&values, 100, 5).unwrap();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 100, scene.rows()));
        let hits = scene
            .paint(&mut buffer, &Theme::default(), Depth::None)
            .unwrap();
        let line = |y| {
            (0..100)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        };
        assert!(line(0).starts_with(" 23.1M"));
        let first = hits
            .iter()
            .find(|hit| {
                hit.id
                    .is_some_and(|path| path.last().is_some_and(|id| id == "full-bar-0"))
            })
            .unwrap();
        assert_eq!(
            first.rect.x, 6,
            "five-character formatted tick plus one separating cell"
        );
        assert!(
            !buffer
                .content
                .iter()
                .any(|cell| matches!(cell.symbol(), "?" | "·"))
        );
        assert_eq!(
            scene.rows(),
            6,
            "five plot rows followed directly by the time axis"
        );
        assert!(line(5).contains("−60m") && line(5).contains("now"));
        assert!(line(4).trim_start().starts_with('0'));
        assert_eq!(values.buckets[0].tokens, Some(23_058_051));
    }

    #[test]
    fn real_template_paints_full_spaces_half_cells_scale_and_baseline() {
        let series = series();
        let scene = Scene::new(&series, 60, 3).unwrap();
        let theme = Theme::new(Base::Tmt);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 60, scene.rows()));
        let hits = scene.paint(&mut buffer, &theme, Depth::TrueColor).unwrap();
        let full = hits
            .iter()
            .find(|hit| {
                hit.id
                    .as_ref()
                    .is_some_and(|id| id.last().unwrap() == "full-bar-3")
            })
            .unwrap();
        let cell = &buffer[(full.rect.x as u16, full.rect.y as u16)];
        assert_eq!(cell.symbol(), " ");
        assert_eq!(
            cell.bg,
            screen::style(&theme, Role::Accent, Depth::TrueColor)
                .fg
                .unwrap()
        );
        assert_ne!(cell.bg, Color::Reset);
        assert!(buffer.content.iter().any(|cell| cell.symbol() == "▄"));
        let line = |y| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        assert!(line(0).contains("100"));
        assert!(line(2).trim_start().starts_with('0'));
        assert_eq!(scene.rows(), 4, "plot plus time axis only");
        assert!(line(3).contains("−60m") && line(3).contains("now"));
        assert_eq!(
            series.buckets[1].tokens,
            Some(1),
            "quantization preserves source values"
        );
    }

    #[test]
    fn theme_depth_clipping_and_unknown_words_use_one_painter() {
        let mut series = series();
        series.buckets[0].tokens = None;
        series.buckets[0].coverage = Coverage::Unknown;
        for base in [Base::Tmt, Base::TmtLight] {
            for depth in [Depth::TrueColor, Depth::Ansi16, Depth::None] {
                for width in [60, 80, 100, 140, 160] {
                    let scene = Scene::new(&series, width, 3).unwrap();
                    let mut buffer = Buffer::empty(Rect::new(2, 1, width, 5));
                    let hits = scene.paint(&mut buffer, &Theme::new(base), depth).unwrap();
                    assert!(hits.iter().all(|hit| hit.rect.x >= 2
                        && hit.rect.y >= 1
                        && hit.rect.x + hit.rect.width as i32 <= 2 + i32::from(width)
                        && hit.rect.y + hit.rect.height as i32 <= 6));
                    assert!(buffer.content.iter().any(|cell| cell.symbol() == "·"));
                    assert_eq!(scene.rows(), 4, "no separate coverage legend row");
                    assert!(
                        !buffer
                            .content
                            .iter()
                            .map(|cell| cell.symbol())
                            .collect::<String>()
                            .contains("unknown")
                    );
                    if depth == Depth::None {
                        assert!(buffer.content.iter().any(|cell| cell.symbol() == "█"));
                        assert!(
                            buffer
                                .content
                                .iter()
                                .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
                        );
                    }
                }
            }
        }
        let scene = Scene::new(&series, 60, 3).unwrap();
        let mut short = Buffer::empty(Rect::new(0, 0, 20, 2));
        assert!(
            scene
                .paint(&mut short, &Theme::default(), Depth::None)
                .unwrap()
                .iter()
                .all(|hit| hit.rect.x + hit.rect.width as i32 <= 20
                    && hit.rect.y + hit.rect.height as i32 <= 2)
        );
        assert!(Scene::new(&series, 60, 25).is_err());
    }

    #[test]
    fn a_monochrome_or_plain_accent_keeps_full_cells_visible() {
        let scene = Scene::new(&series(), 60, 3).unwrap();
        for theme in [
            Theme::new(Base::Mono),
            Theme::default().with(Role::Accent, tmt_cli_style::theme::Paint::Plain),
        ] {
            let mut buffer = Buffer::empty(Rect::new(0, 0, 60, scene.rows()));
            let hits = scene.paint(&mut buffer, &theme, Depth::TrueColor).unwrap();
            let full = hits
                .iter()
                .find(|hit| {
                    hit.id
                        .as_ref()
                        .is_some_and(|id| id.last().unwrap() == "full-bar-3")
                })
                .unwrap();
            let cell = &buffer[(full.rect.x as u16, full.rect.y as u16)];
            assert_eq!(cell.symbol(), " ");
            assert!(cell.modifier.contains(Modifier::REVERSED));
        }
    }

    #[test]
    fn all_unknown_and_known_zero_do_not_show_a_fabricated_token_reading() {
        for (tokens, coverage, expected) in [
            (None, Coverage::Unknown, "·"),
            (Some(0), Coverage::Complete, "0"),
        ] {
            let mut values = series();
            for bucket in &mut values.buckets {
                bucket.tokens = tokens;
                bucket.coverage = coverage;
            }
            let scene = Scene::new(&values, 60, 3).unwrap();
            let mut buffer = Buffer::empty(Rect::new(0, 0, 60, 5));
            scene
                .paint(&mut buffer, &Theme::default(), Depth::TrueColor)
                .unwrap();
            let scale = (0..60).map(|x| buffer[(x, 0)].symbol()).collect::<String>();
            if tokens.is_none() {
                assert!(!scale.contains('·') && !scale.contains('0'), "{scale:?}");
                assert!(buffer.content.iter().any(|cell| cell.symbol() == expected));
            } else {
                assert!(scale.contains(expected), "{scale:?}");
            }
            assert!(!scale.contains("1 tok"));
        }
    }
}

#[cfg(test)]
mod axis_tests {
    use super::super::model::{Bucket, Selection};
    use super::*;
    use ratatui::layout::Rect;
    #[test]
    fn time_axis_and_coverage_marks_share_bar_columns_without_index_labels() {
        for range in [15, 30, 60] {
            for width in [60, 80, 100, 160] {
                let selection = Selection {
                    each_minutes: 5,
                    range_minutes: range,
                };
                let series = Series::new(
                    selection,
                    (0..selection.bars().unwrap())
                        .map(|index| Bucket {
                            from_ms: index as i64 * 300_000,
                            to_ms: (index + 1) as i64 * 300_000,
                            tokens: if index <= 1 { None } else { Some(index as u64) },
                            coverage: if index <= 1 {
                                Coverage::Unknown
                            } else if index == 2 {
                                Coverage::Partial { covered_ms: 50_000 }
                            } else {
                                Coverage::Complete
                            },
                        })
                        .collect(),
                )
                .unwrap();
                let scene = Scene::new(&series, width, 3).unwrap();
                let mut buffer = Buffer::empty(Rect::new(0, 0, width, scene.rows()));
                let hits = scene
                    .paint(&mut buffer, &Theme::default(), Depth::None)
                    .unwrap();
                let line = |y| {
                    (0..width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                };
                let time = line(3);
                assert!(time.contains(&format!("−{range}m")) && time.contains("now"));
                assert!(!time.contains('?'));
                let middle = if range == 15 {
                    "−7.5m".into()
                } else {
                    format!("−{}m", range / 2)
                };
                assert!(time.contains(&middle));
                assert_eq!(
                    scene.rows(),
                    4,
                    "three plot rows then time; no extra dot row"
                );
                for index in 0..series.buckets.len() {
                    let suffix = format!("bar-{index}");
                    let hit = hits
                        .iter()
                        .find(|hit| {
                            hit.rect.y == 2
                                && hit.id.as_ref().is_some_and(|path| {
                                    path.last().is_some_and(|id| id.ends_with(&suffix))
                                })
                        })
                        .unwrap();
                    let columns = hit.rect.x..hit.rect.x + hit.rect.width as i32;
                    if series.buckets[index].tokens.is_none() {
                        assert!(
                            columns
                                .clone()
                                .all(|x| buffer[(x as u16, 2)].symbol() == "·")
                        );
                        for y in 0..2 {
                            assert!(
                                columns
                                    .clone()
                                    .all(|x| buffer[(x as u16, y)].symbol() != "·"),
                                "unknown ghost never fills the upper plot"
                            );
                        }
                    } else {
                        assert!(
                            columns
                                .clone()
                                .all(|x| buffer[(x as u16, 2)].symbol() != "·")
                        );
                    }
                }
                assert!(buffer.content.iter().any(|cell| cell.symbol() == "█"));
                assert!(
                    buffer
                        .content
                        .chunks(usize::from(width))
                        .take(3)
                        .flatten()
                        .all(|cell| cell.symbol() != "?"),
                    "partial coverage is not repeated under each bar"
                );
            }
        }
    }
}
