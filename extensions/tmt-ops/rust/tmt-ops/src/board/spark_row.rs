//! SparkRow: shared provider/member trends, numeric fences and optional quota.
//! Callers supply projected row values; selection, acquisition and effects stay outside.
mod quantize;
pub(in crate::board) use quantize::{glyph, scale};
use ratatui::{layout::Rect, text::Line};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Row {
    pub name: String,
    pub values: Vec<Option<u64>>,
    pub total: String,
    pub share: String,
    pub muted: bool,
    pub quota: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Data {
    pub rows: Vec<Row>,
    pub name_width: u16,
    pub share_caption: String,
    pub show_axis: bool,
    pub axis: [String; 3],
    pub buckets: usize,
    pub no_data: bool,
}

use ratatui::buffer::Buffer;
use serde_json::json;
use std::{collections::BTreeMap, sync::OnceLock};
use tmt_cli_style::{Depth, Role, Theme, grid::Align, theme::screen};
use tmt_tui::{
    binding::{Node, Schema, Schemas, Scopes, Sources, Template},
    geometry, paint, text,
};

const NUMERIC_TRACK_WIDTH: usize = 11;
const MIN_SHARE_WIDTH: usize = 8;
const QUOTA_GAP_WIDTH: usize = 3;
const WIDE_QUOTA_WIDTH: u16 = 49;
const QUOTA_WIDTH: u16 = 37;
const COMPACT_QUOTA_WIDTH: u16 = 15;

const FILE: &str = "ops.board.spark-row.xml";
const MARKUP: &str = r#"<tmt-view version="1" class="w-full">
<tmt-repeat each="$.rows" as="row"><tmt-row id-bind="row.id" class="w-full h-1">
<tmt-repeat each="row.pieces" as="piece"><tmt-text id-bind="piece.id" bind="piece.text" class="shrink-0"/></tmt-repeat>
</tmt-row></tmt-repeat></tmt-view>"#;

struct BindingData;
impl Sources for BindingData {
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
        tmt_tui::binding::compile(FILE, &parsed, &schema, &BindingData)
            .expect("embedded chart schema")
    })
}

fn time_ticks([first, middle, last_label]: &[String; 3], width: usize) -> String {
    let mut cells = vec![' '; width];
    let mut put = |at: usize, label: &str| {
        for (index, ch) in label.chars().enumerate() {
            if let Some(cell) = cells.get_mut(at + index) {
                *cell = ch;
            }
        }
    };
    let first_len = first.chars().count();
    put(0, first);
    let last = width.saturating_sub(last_label.chars().count());
    if last > first_len {
        put(last, last_label);
    }
    let middle_len = middle.chars().count();
    let center = width.saturating_sub(middle_len) / 2;
    if center > first_len && center + middle_len < last {
        put(center, middle);
    }
    cells.into_iter().collect()
}

/// Splits the track into one run per exact bucket. The remainder cells go to
/// buckets spread evenly along the track, so the plot meets the TOK column at
/// every width and no two runs differ by more than one cell.
fn bar_widths(track: usize, buckets: usize) -> Vec<usize> {
    (0..buckets)
        .map(|index| track * (index + 1) / buckets - track * index / buckets)
        .collect()
}

pub struct Scene {
    node: Node,
    rows: u16,
    quota_width: u16,
    spark_width: u16,
    no_data: bool,
    name_width: u16,
}

pub(in crate::board) fn minimum_width(data: &Data) -> usize {
    usize::from(data.name_width)
        + 1
        + NUMERIC_TRACK_WIDTH
        + unicode_width::UnicodeWidthStr::width(data.share_caption.as_str()).max(MIN_SHARE_WIDTH)
        + data.buckets
}
pub(in crate::board) fn quota_width(width: u16, data: &Data) -> u16 {
    let needed = minimum_width(data);
    if width >= 126
        && usize::from(width) >= needed + usize::from(WIDE_QUOTA_WIDTH) + QUOTA_GAP_WIDTH
    {
        WIDE_QUOTA_WIDTH
    } else if usize::from(width) >= needed + usize::from(QUOTA_WIDTH) + QUOTA_GAP_WIDTH {
        QUOTA_WIDTH
    } else if usize::from(width) >= needed + usize::from(COMPACT_QUOTA_WIDTH) + QUOTA_GAP_WIDTH {
        COMPACT_QUOTA_WIDTH
    } else {
        0
    }
}
impl Scene {
    pub fn rows_for(data: &Data) -> u16 {
        (1 + data.rows.len().max(1) + usize::from(!data.no_data && data.show_axis))
            .min(usize::from(u16::MAX)) as u16
    }
    pub fn new(data: &Data, width: u16, quota_width: u16) -> Result<Self, String> {
        if data.buckets == 0 || data.rows.iter().any(|row| row.values.len() != data.buckets) {
            return Err("Spark rows must share one nonempty bucket grid".into());
        }
        let name_width = usize::from(data.name_width);
        let share_width =
            unicode_width::UnicodeWidthStr::width(data.share_caption.as_str()).max(MIN_SHARE_WIDTH);
        let tail = if quota_width > 0 {
            usize::from(quota_width) + QUOTA_GAP_WIDTH
        } else {
            0
        };
        let Some(spark_width) = usize::from(width)
            .checked_sub(name_width + 1 + NUMERIC_TRACK_WIDTH + share_width + tail)
        else {
            return Err("Spark rows need room for names and numeric tracks".into());
        };
        if spark_width < data.buckets {
            return Err("Spark rows need one cell per exact bucket".into());
        }
        let widths = bar_widths(spark_width, data.buckets);
        let scale = quantize::scale(data.rows.iter().flat_map(|row| row.values.iter().copied()))
            .unwrap_or(0);
        let mut heading = vec![
            json!({"id":"heading-padding","text":" ".repeat(name_width+1+spark_width+2)}),
            json!({"id":"heading-total","text":format!("{:>7}","TOK")}),
            json!({"id":"heading-share","text":format!("  {:>share_width$}",data.share_caption)}),
        ];
        if quota_width > 0 {
            heading.push(json!({"id":"heading-quota","text":" ".repeat(usize::from(quota_width)+QUOTA_GAP_WIDTH)}));
        }
        let mut rows = vec![json!({"id":"headings","pieces":heading})];
        let fallback = Row {
            name: String::new(),
            values: vec![None; data.buckets],
            total: "—".into(),
            share: "—".into(),
            muted: true,
            quota: false,
        };
        let visible = if data.rows.is_empty() {
            std::slice::from_ref(&fallback)
        } else {
            data.rows.as_slice()
        };
        for (provider, row) in visible.iter().enumerate() {
            let prefix = if row.muted { "muted-" } else { "" };
            let name = crate::board::view::fit(&row.name, name_width);
            let mut pieces =
                vec![json!({"id":format!("{prefix}name-{provider}"),"text":format!("{name} ")})];
            for (index, reading) in row.values.iter().enumerate() {
                let glyph = if data.no_data
                    || row.total == "—" && row.values.iter().all(Option::is_none)
                {
                    " "
                } else {
                    quantize::glyph(*reading, scale)
                };
                pieces.push(json!({"id":format!("spark-{provider}-bar-{index}"),"text":glyph.repeat(widths[index])}));
            }
            let total = text::fit_line(
                &row.total,
                7,
                tmt_tui::style::TextFlow::Truncate,
                Align::Right,
            );
            pieces.push(
                json!({"id":format!("{prefix}total-{provider}"),"text":format!("  {total}  ")}),
            );
            pieces.push(json!({"id":format!("share-{provider}"),"text":format!("{:>share_width$}",row.share)}));
            if quota_width > 0 {
                pieces.push(json!({"id":"quota-gap","text":"   "}));
                pieces.push(json!({"id":format!("{}quota-{provider}",if row.quota {""}else{"no-"}),"text":" ".repeat(usize::from(quota_width))}));
            }
            rows.push(json!({"id":format!("provider-{provider}"),"pieces":pieces}));
        }
        if !data.no_data && data.show_axis {
            rows.push(json!({"id":"time","pieces":[{"id":"axis-padding","text":" ".repeat(name_width+1)},{"id":"time-ticks","text":time_ticks(&data.axis,spark_width)}]}));
        }
        let row_count = rows.len() as u16;
        let node = template()
            .materialize(FILE, &json!({"rows":rows}), &BindingData)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            node,
            rows: row_count,
            quota_width,
            spark_width: spark_width as u16,
            no_data: data.no_data,
            name_width: data.name_width,
        })
    }
    pub fn quota_width(&self) -> u16 {
        self.quota_width
    }
    pub fn rows(&self) -> u16 {
        self.rows
    }
    pub fn paint<'a>(
        &'a self,
        buffer: &mut Buffer,
        theme: &Theme,
        depth: Depth,
        quotas: &[Option<Line<'static>>],
        inspection: Option<&str>,
    ) -> Result<Vec<paint::Hit<'a>>, String> {
        let area = buffer.area;
        let mut cells = geometry::layout(&self.node, [area.width, area.height], text::measure)?;
        for cell in &mut cells {
            for rect in [&mut cell.rect, &mut cell.content, &mut cell.clip] {
                rect.x += i32::from(area.x);
                rect.y += i32::from(area.y);
            }
        }
        let hits = paint::paint_with(&cells, buffer, |index, _, _| {
            let id = cells[index]
                .node
                .id
                .as_ref()
                .and_then(|id| id.last())
                .map(String::as_str)
                .unwrap_or("");
            let role = if id.starts_with("spark-")
                || id.starts_with("share-")
                || id.starts_with("muted-")
                || id.starts_with("heading-")
                || id == "time-ticks"
            {
                Role::Muted
            } else {
                Role::Text
            };
            (screen::style(theme, role, depth), Align::Left)
        });
        for hit in &hits {
            let Some(index) = hit
                .id
                .and_then(|id| id.last())
                .and_then(|id| id.strip_prefix("quota-"))
                .and_then(|index| index.parse::<usize>().ok())
            else {
                continue;
            };
            if let Some(Some(line)) = quotas.get(index) {
                let rect = Rect::new(
                    hit.rect.x as u16,
                    hit.rect.y as u16,
                    hit.rect.width as u16,
                    hit.rect.height as u16,
                )
                .intersection(area);
                tmt_tui::components::strip::paint_left(buffer, rect, line.clone());
            }
        }
        if let Some(note) = inspection
            && let Some(axis) = hits.iter().find(|hit| {
                hit.id
                    .and_then(|path| path.last())
                    .is_some_and(|id| id == "time-ticks")
            })
        {
            tmt_tui::components::strip::paint_left(
                buffer,
                Rect::new(area.x, axis.rect.y as u16, area.width, 1).intersection(area),
                Line::styled(
                    crate::board::view::fit(note, usize::from(area.width)),
                    screen::style(theme, Role::Muted, depth),
                ),
            );
        }
        if self.no_data {
            crate::board::empty_state::paint(
                buffer,
                Rect::new(
                    area.x + self.name_width + 1,
                    area.y + 1,
                    self.spark_width,
                    1,
                )
                .intersection(area),
                "No token observations yet",
                None,
                crate::look::Look {
                    theme: *theme,
                    depth,
                },
            );
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{layout::Rect, style::Color};
    use tmt_cli_style::theme::Base;
    fn data() -> Data {
        Data {
            rows: [
                ("Claude", 60, "720", "60%", false),
                ("Codex", 30, "360", "30%", false),
                ("unknown", 10, "120", "10%", true),
            ]
            .into_iter()
            .map(|(name, value, total, share, muted)| Row {
                name: name.into(),
                values: vec![Some(value); 12],
                total: total.into(),
                share: share.into(),
                muted,
                quota: !muted,
            })
            .collect(),
            name_width: 8,
            share_caption: "% of all".into(),
            show_axis: true,
            axis: ["−60m", "−30m", "now"].map(str::to_owned),
            buckets: 12,
            no_data: false,
        }
    }
    fn line(buffer: &Buffer, y: u16) -> String {
        (buffer.area.x..buffer.area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }
    #[test]
    fn provider_rows_at_103_have_exact_numeric_fences_stretch_and_neutral_roles() {
        let source = data();
        let scene = Scene::new(&source, 103, 0).unwrap();
        assert_eq!(scene.rows(), 5);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 103, 5));
        let hits = scene
            .paint(&mut buffer, &Theme::default(), Depth::TrueColor, &[], None)
            .unwrap();
        assert_eq!(
            line(&buffer, 1),
            format!("Claude   {}  {:>7}  {:>8}", "█".repeat(75), "720", "60%")
        );
        assert_eq!(
            line(&buffer, 2),
            format!("Codex    {}  {:>7}  {:>8}", "▄".repeat(75), "360", "30%")
        );
        assert_eq!(
            line(&buffer, 3),
            format!("unknown  {}  {:>7}  {:>8}", "▂".repeat(75), "120", "10%")
        );
        let axis = line(&buffer, 4);
        assert!(axis.starts_with("         −60m"));
        assert!(axis.chars().take(84).collect::<String>().ends_with("now"));
        assert_eq!(axis.chars().skip(84).collect::<String>(), " ".repeat(19));
        assert!(line(&buffer, 0).ends_with("TOK  % of all"));
        for x in 9..84 {
            assert_eq!(buffer[(x, 1)].fg, buffer[(x, 2)].fg);
        }
        assert_eq!(
            hits.iter()
                .filter(|hit| hit
                    .id
                    .as_ref()
                    .is_some_and(|id| id.last().unwrap() == "spark-0-bar-0"))
                .count(),
            1
        );
    }
    #[test]
    fn plot_fills_its_whole_track_and_meets_tok_at_every_width_and_bucket_count() {
        for buckets in [12, 24] {
            for width in [80_u16, 103, 126, 160, 200] {
                let mut source = data();
                source.buckets = buckets;
                for row in &mut source.rows {
                    row.values = vec![Some(60); buckets];
                }
                let quota = quota_width(width, &source);
                let scene = Scene::new(&source, width, quota).unwrap();
                let tail = if quota > 0 {
                    usize::from(quota) + QUOTA_GAP_WIDTH
                } else {
                    0
                };
                let track = usize::from(width) - (9 + NUMERIC_TRACK_WIDTH + 8 + tail);
                let mut buffer = Buffer::empty(Rect::new(0, 0, width, 5));
                let hits = scene
                    .paint(&mut buffer, &Theme::default(), Depth::TrueColor, &[], None)
                    .unwrap();
                let case = format!("{buckets} buckets at {width} (quota {quota})");
                let heading: Vec<char> = line(&buffer, 0).chars().collect();
                for (y, total) in [(1, "720"), (2, "360"), (3, "120")] {
                    let row: Vec<char> = line(&buffer, y).chars().collect();
                    assert!(row[9..9 + track].iter().all(|ch| *ch != ' '), "{case}");
                    assert_eq!(
                        row[9 + track..9 + track + 9].iter().collect::<String>(),
                        format!("  {total:>7}"),
                        "{case}"
                    );
                }
                assert_eq!(
                    heading[9 + track + 2..9 + track + 9]
                        .iter()
                        .collect::<String>(),
                    format!("{:>7}", "TOK"),
                    "{case}"
                );
                let axis: Vec<char> = line(&buffer, 4).chars().collect();
                assert_eq!(
                    axis[9 + track - 3..9 + track].iter().collect::<String>(),
                    "now",
                    "{case}"
                );
                assert!(axis[9 + track..].iter().all(|ch| *ch == ' '), "{case}");
                let mut runs: Vec<(i32, usize)> = hits
                    .iter()
                    .filter(|hit| {
                        hit.id
                            .as_ref()
                            .is_some_and(|id| id.last().unwrap().starts_with("spark-0-bar-"))
                    })
                    .map(|hit| (hit.rect.x, hit.rect.width as usize))
                    .collect();
                runs.sort();
                assert_eq!(runs.len(), buckets, "{case}");
                assert_eq!(runs.iter().map(|run| run.1).sum::<usize>(), track, "{case}");
                let (low, high) = (
                    runs.iter().map(|run| run.1).min().unwrap(),
                    runs.iter().map(|run| run.1).max().unwrap(),
                );
                assert!(high - low <= 1, "{case}: widths {low}..{high}");
                let mut next = 9;
                for (x, run) in runs {
                    assert_eq!(x, next, "{case}");
                    next += run as i32;
                }
            }
        }
    }
    #[test]
    fn rows_clip_hits_and_preserve_unknown_zero_and_no_color_at_supported_widths() {
        assert!(Scene::new(&data(), 38, 0).is_err());
        for width in [40, 60, 80, 100, 103, 148, 160] {
            for base in [Base::Tmt, Base::TmtLight] {
                for depth in [Depth::TrueColor, Depth::None] {
                    let mut source = data();
                    for row in &mut source.rows {
                        row.values[0] = None;
                        row.values[1] = Some(0);
                    }
                    let scene = Scene::new(&source, width, 0).unwrap();
                    let mut buffer = Buffer::empty(Rect::new(2, 1, width, 5));
                    let hits = scene
                        .paint(&mut buffer, &Theme::new(base), depth, &[], None)
                        .unwrap();
                    assert!(hits.iter().all(|hit| hit.rect.x >= 2
                        && hit.rect.y >= 1
                        && hit.rect.x + hit.rect.width as i32 <= i32::from(width) + 2
                        && hit.rect.y + hit.rect.height as i32 <= 6));
                    assert!(buffer.content.iter().any(|cell| cell.symbol() == "·"));
                    assert!(
                        !buffer
                            .content
                            .iter()
                            .any(|cell| matches!(cell.symbol(), "▒" | "░"))
                    );
                    if depth == Depth::None {
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
    }
    #[test]
    fn no_data_keeps_names_quota_and_dashes_with_one_empty_line_and_no_axis() {
        let mut source = data();
        source.rows.truncate(2);
        source.no_data = true;
        for row in &mut source.rows {
            row.values.fill(None);
            row.total = "—".into();
            row.share = "—".into();
        }
        let scene = Scene::new(&source, 102, quota_width(102, &source)).unwrap();
        assert_eq!(scene.rows(), 3);
        let mut buffer = Buffer::empty(Rect::new(1, 5, 102, 3));
        scene
            .paint(
                &mut buffer,
                &Theme::default(),
                Depth::TrueColor,
                &[
                    Some(Line::raw("████████░░ 82%  —")),
                    Some(Line::raw("█████████░ 88%  —")),
                ],
                None,
            )
            .unwrap();
        let shown = (5..8)
            .map(|y| line(&buffer, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            shown.contains("Claude")
                && shown.contains("Codex")
                && shown.contains("82%")
                && shown.contains("88%")
        );
        assert_eq!(shown.matches("No token observations yet").count(), 1);
        assert!(!shown.contains("−60m") && !shown.contains("unknown") && !shown.contains('?'));
        for y in [6, 7] {
            assert_eq!(
                line(&buffer, y)
                    .chars()
                    .skip(45)
                    .take(7)
                    .collect::<String>(),
                format!("{:>7}", "—")
            );
        }
        let mut zero = data();
        for row in &mut zero.rows {
            row.values.fill(Some(0));
            row.total = "0".into();
            row.share = "0%".into();
        }
        let scene = Scene::new(&zero, 103, 0).unwrap();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 103, 5));
        scene
            .paint(&mut buffer, &Theme::default(), Depth::TrueColor, &[], None)
            .unwrap();
        assert!(line(&buffer, 1).contains("0%"));
        assert!(
            !(0..5)
                .map(|y| line(&buffer, y))
                .collect::<Vec<_>>()
                .join("\n")
                .contains("No token observations yet")
        );
    }
}
