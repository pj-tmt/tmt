//! The two one-line targets between the leads and the squads: the `→ all leads`
//! audience line and the cron line. Each is one selectable row; the cron line's
//! text comes from the cron projection, the audience line's from this module.
use super::scene::{self, Part};
use crate::{board::app::App, board::view::fit, look::Look};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
};
use serde_json::{Value, json};
use std::{ops::Range, sync::OnceLock};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::binding::{Schema, Template};

const FILE: &str = "squad.home.row.xml";

/// An optional blank line, the row (its pieces, then a filler that carries the
/// row's base style to the edge), and the lines reserved under it.
const MARKUP: &str = r#"<tmt-view version="1">
<tmt-repeat each="$.gap" as="gap"><tmt-text id="gap" class="w-full h-1"/></tmt-repeat>
<tmt-row id="line" class="w-full h-1">
<tmt-repeat each="$.pieces" as="piece"><tmt-text id-bind="piece.id" bind="piece.text" token-bind="piece.role" class="shrink-0"/></tmt-repeat>
<tmt-text id="fill" class="grow h-1"/>
</tmt-row>
<tmt-repeat each="$.reserve" as="line"><tmt-text id-bind="line.id" class="w-full h-1"/></tmt-repeat>
</tmt-view>"#;

fn template() -> &'static Template<()> {
    static TEMPLATE: OnceLock<Template<()>> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        use std::collections::BTreeMap;
        let object = |fields: Vec<(&str, Schema)>| {
            Schema::Object(
                fields
                    .into_iter()
                    .map(|(name, schema)| (name.to_owned(), schema))
                    .collect::<BTreeMap<_, _>>(),
            )
        };
        let list = |item| Schema::Collection(Box::new(item));
        let schema = object(vec![
            ("gap", list(object(vec![]))),
            (
                "pieces",
                list(object(vec![
                    ("id", Schema::StableId),
                    ("text", Schema::Scalar),
                    ("role", Schema::Scalar),
                ])),
            ),
            ("reserve", list(object(vec![("id", Schema::StableId)]))),
        ]);
        scene::compile(FILE, MARKUP, &schema)
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    AllLeads,
    Cron,
}

pub(super) struct Block {
    pub lines: Vec<Line<'static>>,
    /// The row itself, relative to the block.
    pub row: usize,
    pub reserve: Option<Range<usize>>,
}

/// The row for the entry at `index`. The audience line reserves composer lines
/// under it; the cron line has no composer.
pub(super) fn paint(app: &App, look: Look, area: Rect, kind: Kind, index: usize) -> Block {
    let pieces: Vec<(String, Role)> = match kind {
        Kind::AllLeads => vec![(
            fit(
                "→ all leads  A to write · @ to pick",
                usize::from(area.width),
            ),
            Role::Muted,
        )],
        Kind::Cron => {
            crate::board::cronboard::home_pieces(&app.cron, app.cron.now_ms(), area.width)
                .expect("a cron target exists only with a read or its failure")
        }
    };
    let reserved = if kind == Kind::AllLeads {
        crate::board::view::waiting::reserved_lines(app, index, area).unwrap_or_default()
    } else {
        0
    };
    let selected = index == app.selected;
    let data: Value = json!({
        "gap": if kind == Kind::Cron { vec![json!({})] } else { Vec::new() },
        "pieces": pieces.iter().enumerate().map(|(at, (text, role))| {
            json!({"id": format!("p{at}"), "text": text, "role": role.name()})
        }).collect::<Vec<_>>(),
        "reserve": (0..reserved).map(|at| json!({"id": format!("reserve-{at}")})).collect::<Vec<_>>(),
    });
    let painted = scene::paint(
        FILE,
        template(),
        &data,
        area.width,
        &mut |Part { id, role, .. }| {
            let part = id.and_then(<[String]>::last).map(String::as_str);
            let base = if selected {
                look.selection().add_modifier(Modifier::BOLD)
            } else {
                look.role(Role::Text)
            };
            let style = match (kind, part) {
                // The audience line is one styled string.
                (Kind::AllLeads, Some(piece)) if piece.starts_with('p') => {
                    if selected {
                        base
                    } else {
                        look.role(role)
                    }
                }
                (Kind::Cron, Some(piece)) if piece.starts_with('p') => {
                    base.patch(look.row_span(selected, look.role(role), false))
                }
                (Kind::Cron, Some("fill")) => base,
                _ => Style::new(),
            };
            (style, Align::Left)
        },
    );
    let row = painted.lines(&["line"]).expect("the row is a scene block");
    Block {
        reserve: (reserved > 0).then(|| row.end..row.end + reserved),
        row: row.start,
        lines: painted.lines,
    }
}
