//! Squad presentation adapter; projected fields remain display values.
#[cfg(test)]
use crate::{
    rows,
    source::{ColumnSource, Format},
    squad::Member,
};
use std::collections::{BTreeMap, BTreeSet};
use tmt_tui::binding::{Schema, Schemas, Scopes, Sources};
use unicode_width::UnicodeWidthStr;

#[cfg(test)]
struct Adapter<'a> {
    members: BTreeMap<&'a str, &'a Member>,
    provided: &'a [String],
    now_ms: u64,
}
#[cfg(test)]
impl Sources for Adapter<'_> {
    type Source = (ColumnSource, Format);
    fn compile(
        &self,
        from: &str,
        format: &str,
        scopes: &Schemas<'_>,
    ) -> Result<Self::Source, String> {
        if !matches!(scopes.get("row"), Some(Schema::Object(fields)) if matches!(fields.get("id"), Some(Schema::StableId)))
        {
            return Err("Squad sources require lexical row.id: StableId".into());
        }
        let source = ColumnSource::parse(from, rows::field_name, |name| {
            self.provided.iter().any(|v| v == name)
        })
        .ok_or_else(|| format!("unknown Squad source {from:?}"))?;
        let format =
            Format::parse(format).ok_or_else(|| format!("unknown Squad format {format:?}"))?;
        Ok((source, format))
    }
    fn resolve(
        &self,
        (source, format): &Self::Source,
        scopes: &Scopes<'_>,
    ) -> Result<Option<String>, String> {
        let member = scopes
            .get("row")
            .and_then(|row| row["id"].as_str())
            .and_then(|id| self.members.get(id))
            .ok_or("Squad source requires an acquired member matching row.id")?;
        Ok(source.value(member, *format, self.now_ms))
    }
}

#[test]
fn binding_reuses_acquired_sources_and_never_reformats_projected_fields() {
    use serde_json::{Value, json};
    use tmt_tui::{binding, parse};
    let member = Member {
        lead_marker: None,
        id: "member-a".into(),
        name: "rin".into(),
        lifetime: "saved".into(),
        presence: "active".into(),
        pane: Value::Null,
        activity: Value::Null,
        fields: BTreeMap::from([("pr_state".into(), "?".into())]),
        meta: BTreeMap::from([("seen".into(), "60000".into())]),
        numbers: BTreeMap::new(),
        colors: BTreeMap::new(),
        failed: ["pr_state".into()].into(),
        seen: json!({"resume": {"usage": {"tokens": 487000}}}),
    };
    let provided = vec!["pr_state".into()];
    let adapter = Adapter {
        members: BTreeMap::from([(member.id.as_str(), &member)]),
        provided: &provided,
        now_ms: 120000,
    };
    let row = Schema::Object(BTreeMap::from([
        ("id".into(), Schema::StableId),
        ("shown".into(), Schema::Scalar),
    ]));
    let schema = Schema::Object(BTreeMap::from([(
        "rows".into(),
        Schema::Collection(Box::new(row)),
    )]));
    for (from, format, expected) in [
        ("session.usage.tokens", "tokens", Some("487k")),
        ("session.usage.tokens", "count", Some("487,000")),
        ("meta.seen", "age", Some("1m")),
        ("member", "text", Some("rin")),
        ("fields.pr_state", "count", Some("?")),
        ("cwd", "text", None),
    ] {
        let xml = format!(
            "<tmt-view version=\"1\"><tmt-repeat each=\"$.rows\" as=\"row\"><tmt-cell from=\"{from}\" format=\"{format}\"/><tmt-cell bind=\"row.shown\"/></tmt-repeat></tmt-view>"
        );
        let template = binding::compile(
            "squad.xml",
            &parse("squad.xml", &xml).unwrap(),
            &schema,
            &adapter,
        )
        .unwrap();
        let node = template
            .materialize(
                "squad.xml",
                &json!({"rows": [{"id": member.id, "shown": "487k"}]}),
                &adapter,
            )
            .unwrap();
        let configured: toml_edit::DocumentMut = format!(
            "[p]\nrows.columns=[{{name='shown',from='{from}',format='{format}'}}]\nfields.pr_state={{}}\n"
        ).parse().unwrap();
        let rows = rows::read(configured["p"].as_table_like(), "p").unwrap();
        let column = &rows.columns[0];
        assert_eq!(
            node.children[0].text,
            column
                .source()
                .unwrap()
                .value(&member, column.format, adapter.now_ms)
        );
        assert_eq!(node.children[0].text.as_deref(), expected);
        assert_eq!(node.children[1].text.as_deref(), Some("487k"));
        assert!(
            template
                .materialize(
                    "squad.xml",
                    &json!({"rows": [{"id": "missing", "shown": "487k"}]}),
                    &adapter
                )
                .is_err()
        );
    }
    for attrs in [
        "from=\"unknown\"",
        "from=\"member\" format=\"unknown\"",
        "from=\"fields.missing\"",
        "bind=\"row.shown\" format=\"tokens\"",
    ] {
        let xml = format!(
            "<tmt-view version=\"1\"><tmt-repeat each=\"$.rows\" as=\"row\"><tmt-cell {attrs}/></tmt-repeat></tmt-view>"
        );
        assert!(
            binding::compile(
                "squad.xml",
                &parse("squad.xml", &xml).unwrap(),
                &schema,
                &adapter
            )
            .is_err()
        );
    }
}

/// Read already projected display values; never invoke the source registry here.
pub fn value<'a>(row: &'a serde_json::Value, field: &str) -> Option<&'a str> {
    match field {
        "member" => row["name"].as_str(),
        "pending" => row["pending"].as_str(),
        field => row["fields"][field].as_str(),
    }
}

/// One admitted scene per occurrence. Section slots are authored layout scopes,
/// not member positions; rows without a member UUID remain non-actionable.
pub fn row_values(
    rows: &crate::rows::Rows,
    tab: &str,
    occurrences: Vec<(usize, &serde_json::Value)>,
) -> Result<Vec<tmt_tui::binding::Node>, String> {
    use serde_json::{Value, json};
    use tmt_tui::{binding, parse};
    let mut fields = BTreeMap::from([
        ("tab".into(), Schema::StableId),
        ("scope".into(), Schema::StableId),
        ("squad".into(), Schema::StableId),
        ("id".into(), Schema::StableId),
    ]);
    let mut body = String::new();
    for (line, cells) in rows.lines.iter().enumerate() {
        body.push_str(&format!("<tmt-row id='line-{line}' class='grid'>"));
        let mut position = 0;
        for (at, cell) in cells.iter().enumerate() {
            let key = format!("v{line}_{at}");
            fields.insert(key.clone(), Schema::Scalar);
            let token = cell
                .token
                .map_or_else(String::new, |role| format!(" token='{}'", role.name()));
            body.push_str(&format!(
                "<tmt-cell id='column-{position}' class='col-span-{}' bind='$.{key}'{token}/>",
                cell.span
            ));
            position += cell.span;
        }
        body.push_str("</tmt-row>");
    }
    let schema = Schema::Object(fields);
    let identified = format!(
        "<tmt-view version='1' id-bind='$.tab'><tmt-col id-bind='$.scope'><tmt-col id-bind='$.squad'><tmt-row id-bind='$.id' row-bind='$.id'>{body}</tmt-row></tmt-col></tmt-col></tmt-view>"
    );
    let mut plain = format!(
        "<tmt-view version='1'><tmt-col><tmt-col><tmt-row>{body}</tmt-row></tmt-col></tmt-col></tmt-view>"
    );
    // The static line/column IDs are meaningful only within a stable member scope.
    for (line, cells) in rows.lines.iter().enumerate() {
        plain = plain.replace(&format!(" id='line-{line}'"), "");
        let mut position = 0;
        for cell in cells {
            plain = plain.replace(&format!(" id='column-{position}'"), "");
            position += cell.span;
        }
    }
    let templates = [identified, plain]
        .map(|xml| {
            binding::compile(
                "squad.rows",
                &parse("squad.rows", &xml)?,
                &schema,
                &Projected,
            )
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut ids = BTreeSet::new();
    occurrences.into_iter().map(|(section, row)| {
        let mut data = json!({"tab": format!("tab:{tab}"), "scope": format!("section-{section}"), "squad": format!("squad:{}", row["squad"].as_str().unwrap_or(tab)), "id": row["id"]});
        for (line, cells) in rows.lines.iter().enumerate() {
            for (at, cell) in cells.iter().enumerate() {
                data[format!("v{line}_{at}")] = cell.field.as_deref()
                    .and_then(|field| value(row, field)).map_or(Value::Null, |v| json!(v));
            }
        }
        let mut scene = templates[usize::from(row["id"].is_null())]
            .materialize("squad.rows", &data, &Projected).map_err(|e| e.to_string())?;
        let mut node = scene.children.remove(0).children.remove(0).children.remove(0);
        if let Some(id) = &node.id && !ids.insert(id.clone()) {
            return Err(format!("squad.rows: duplicate member occurrence {id:?}"));
        }
        for (cells, configured) in node.children.iter_mut().zip(&rows.lines) {
            let mut position = 0;
            for (cell, configured) in cells.children.iter_mut().zip(configured) {
                cell.style.text_flow = flow(&rows.columns[position]);
                position += configured.span;
            }
        }
        Ok(node)
    }).collect()
}

/// Preserve configured board text flow while sharing markup's grapheme owner.
pub fn flow(column: &crate::rows::Column) -> tmt_tui::style::TextFlow {
    use tmt_cli_style::grid::{Overflow, Truncate};
    use tmt_tui::style::TextFlow;
    match column.overflow.unwrap_or_default() {
        Overflow::Wrap { max_lines } => TextFlow::Clamp(u16::from(max_lines)),
        Overflow::Ellipsis if column.truncate == Truncate::Middle => TextFlow::Middle,
        Overflow::Ellipsis => TextFlow::Truncate,
    }
}

/// Resolved Taffy boxes, not a second track model. Original positions survive hiding.
#[derive(Debug, Clone, Copy)]
pub struct BoxWidth {
    pub visible: usize,
    pub text: u16,
    pub cut: bool,
}
#[derive(Debug)]
pub struct Grid {
    pub columns: Vec<Option<usize>>,
    boxes: BTreeMap<(usize, usize), BoxWidth>,
}
struct Projected;
impl Sources for Projected {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("row compilation binds projected display values, not sources".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("row compilation does not acquire or format sources".into())
    }
}
impl Grid {
    pub fn span(&self, range: std::ops::Range<usize>) -> Option<BoxWidth> {
        self.boxes.get(&(range.start, range.end)).copied()
    }
    /// Layout the covered shared tracks and every configured span once per viewport.
    pub fn compile(
        rows: &crate::rows::Rows,
        natural: impl Fn(usize) -> usize,
        available: usize,
    ) -> Result<Self, String> {
        use tmt_cli_style::grid::Basis;
        use tmt_tui::{
            binding, geometry,
            style::{Breadth, GridTrack},
        };
        let available = available.min(usize::from(u16::MAX));
        let count = rows.covered_tracks();
        let natural: Vec<_> = (0..count)
            .map(|i| natural(i).min(usize::from(u16::MAX)))
            .collect();
        let bases: Vec<_> = rows.columns[..count]
            .iter()
            .map(|c| {
                let width = match c.width {
                    Some(Basis::Cells(n)) => Some(n),
                    Some(Basis::Percent(p)) => Some(available * usize::from(p) / 100),
                    None => c
                        .min
                        .map(usize::from)
                        .or_else(|| (c.grow > 0).then_some(crate::rows::NARROWEST)),
                };
                width.map_or(Breadth::Auto, |n| {
                    Breadth::Cells(n.min(c.max.map_or(usize::from(u16::MAX), usize::from)).max(
                        c.min.map_or_else(
                            || {
                                if c.grow > 0 {
                                    crate::rows::NARROWEST
                                } else {
                                    0
                                }
                            },
                            usize::from,
                        ),
                    ) as u16)
                })
            })
            .collect();
        let mut shown = vec![true; count];
        loop {
            let visible: Vec<_> = (0..count).filter(|i| shown[*i]).collect();
            let required = visible
                .iter()
                .enumerate()
                .map(|(at, &i)| {
                    let base = match bases[i] {
                        Breadth::Cells(n) => usize::from(n),
                        _ => rows.columns[i]
                            .max
                            .map_or(natural[i], |max| natural[i].min(usize::from(max))),
                    };
                    // Optional tracks fit whole before grid sizing; only
                    // non-priority overflow may keep a four-cell right cut.
                    if at + 1 == visible.len() && rows.columns[i].priority.is_none() {
                        base.min(4)
                    } else {
                        base
                    }
                })
                .sum::<usize>()
                + visible.len().saturating_sub(1);
            if required <= available {
                break;
            }
            let drop = (0..count)
                .filter(|i| shown[*i] && rows.columns[*i].priority.is_some())
                .max_by_key(|i| (rows.columns[*i].priority, *i));
            match drop {
                Some(i) => shown[i] = false,
                None => break,
            }
        }
        let tracks: Vec<_> = (0..count)
            .filter(|i| shown[*i])
            .map(|i| {
                let c = &rows.columns[i];
                let max = match (c.width, c.grow, c.max) {
                    (Some(_), 0, _) | (_, 0, None) => bases[i],
                    (_, _, Some(max)) => Breadth::Cells(match bases[i] {
                        Breadth::Cells(min) => max.max(min),
                        _ => max,
                    }),
                    _ => Breadth::Fraction(c.grow),
                };
                GridTrack { min: bases[i], max }
            })
            .collect();
        if tracks.is_empty() {
            return Ok(Self {
                columns: vec![None; rows.columns.len()],
                boxes: BTreeMap::new(),
            });
        }
        // Styles/structure/bind budgets pass the same admission as author templates.
        // Computed clamp bases are typed viewport inputs, not arbitrary CSS strings.
        let mut xml = String::from("<tmt-view version='1' class='grid gap-x-1'>");
        let mut schema = BTreeMap::new();
        let mut data = serde_json::Map::new();
        let mut ranges = Vec::new();
        for i in (0..count).filter(|i| shown[*i]) {
            let key = format!("c{i}");
            xml.push_str(&format!(
                "<tmt-cell id='{key}' class='h-1 min-w-0' bind='$.{key}'/>"
            ));
            schema.insert(key.clone(), Schema::Scalar);
            data.insert(key, serde_json::Value::String("x".repeat(natural[i])));
            ranges.push((i, i + 1));
        }
        for (line, cells) in rows.lines.iter().enumerate() {
            let mut position = 0;
            for (at, cell) in cells.iter().enumerate() {
                let end = position + cell.span;
                let span = shown[position..end].iter().filter(|v| **v).count();
                if span > 0 {
                    xml.push_str(&format!(
                        "<tmt-cell id='line-{line}-cell-{at}' class='h-1 col-span-{span}'/>"
                    ));
                    ranges.push((position, end));
                }
                position = end;
            }
            let tail = shown[position..].iter().filter(|v| **v).count();
            if tail > 0 {
                xml.push_str(&format!("<tmt-cell class='h-1 col-span-{tail}'/>"));
                ranges.push((position, count));
            }
        }
        xml.push_str("</tmt-view>");
        let template = binding::compile(
            "squad.rows",
            &tmt_tui::parse("squad.rows", &xml).map_err(|e| e.to_string())?,
            &Schema::Object(schema),
            &Projected,
        )
        .map_err(|e| e.to_string())?;
        let mut scene = template
            .materialize("squad.rows", &serde_json::Value::Object(data), &Projected)
            .map_err(|e| e.to_string())?;
        scene.style.columns = tracks.into();
        let cells = geometry::layout(
            &scene,
            [available as u16, rows.lines.len() as u16 + 1],
            tmt_tui::text::measure,
        )?;
        let mut boxes = BTreeMap::new();
        for (cell, range) in cells[1..].iter().zip(ranges) {
            if cell.clip.width > 0 {
                boxes.insert(
                    range,
                    BoxWidth {
                        visible: cell.clip.width as usize,
                        text: cell.text_width,
                        cut: cell.cut,
                    },
                );
            }
        }
        let columns = (0..rows.columns.len())
            .map(|i| boxes.get(&(i, i + 1)).map(|b| b.visible))
            .collect();
        Ok(Self { columns, boxes })
    }
}

/// Fit logical lines before the viewport cut; spare rounded cells remain blanks.
pub fn fitted(
    value: &str,
    box_width: BoxWidth,
    flow: tmt_tui::style::TextFlow,
    align: tmt_cli_style::grid::Align,
) -> Vec<String> {
    use tmt_tui::{style::TextFlow, text};
    text::fit_lines(value, box_width.text, flow, align)
        .into_iter()
        .map(|logical| {
            let mut visible = if box_width.cut {
                text::fit_line(
                    &logical,
                    box_width.visible as u16,
                    if flow == TextFlow::Middle {
                        TextFlow::Middle
                    } else {
                        TextFlow::Truncate
                    },
                    align,
                )
            } else {
                logical
            };
            visible.push_str(&" ".repeat(box_width.visible.saturating_sub(visible.width())));
            visible
        })
        .collect()
}

#[cfg(test)]
mod tests;
