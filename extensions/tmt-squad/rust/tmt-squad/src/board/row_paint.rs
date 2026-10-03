//! Scalar paint adapts admitted identities and solved boxes; App/Scrolls own input.
use super::app::{App, Hit, Item};
use crate::{look::Look, markup, rows::Rows};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::{
    binding::Node,
    geometry::{Cell, Rect as BoxRect},
    style::TextFlow,
};

struct Part {
    node: Node,
    rect: BoxRect,
    width: u16,
    cut: bool,
    parent: Option<usize>,
    row: Option<usize>,
    emphasize: bool,
    align: Align,
}
pub(super) struct RowPaint {
    parts: Vec<Part>,
    pub starts: Vec<usize>,
    pub ends: Vec<usize>,
    pub height: usize,
    ages: Vec<bool>,
}
impl RowPaint {
    fn add(
        &mut self,
        node: Node,
        rect: (usize, usize, usize, usize),
        parent: Option<usize>,
    ) -> usize {
        let index = self.parts.len();
        let (x, y, width, height) = rect;
        self.parts.push(Part {
            node,
            rect: BoxRect {
                x: x as i32,
                y: y as i32,
                width: width as u32,
                height: height as u32,
            },
            width: width as u16,
            cut: false,
            parent,
            row: parent.and_then(|index| self.parts[index].row),
            emphasize: false,
            align: Align::Left,
        });
        index
    }
    fn label(
        &mut self,
        text: String,
        y: usize,
        width: usize,
        role: Option<Role>,
        parent: Option<usize>,
    ) -> usize {
        static TEXT: std::sync::OnceLock<tmt_tui::style::CellStyle> = std::sync::OnceLock::new();
        let mut style = TEXT
            .get_or_init(|| {
                tmt_tui::parse("squad.rows", "<tmt-view version='1'><tmt-text/></tmt-view>")
                    .expect("literal text prototype")
                    .children
                    .remove(0)
                    .style
            })
            .clone();
        style.token = role;
        let node = Node {
            kind: tmt_tui::Kind::Text,
            style,
            id: None,
            row_id: None,
            selected: false,
            text: Some(text),
            children: Vec::new(),
        };
        self.add(node, (0, y, width, 1), parent)
    }
    pub fn build(
        app: &App,
        rows: &Rows,
        layout: &markup::Grid,
        admitted: &[Node],
        width: usize,
    ) -> Self {
        use unicode_width::UnicodeWidthStr;
        let mut scene = Self {
            parts: Vec::new(),
            starts: Vec::new(),
            ends: Vec::new(),
            height: 1,
            ages: Vec::new(),
        };
        let header = rows
            .columns
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                layout.span(i..i + 1).map(|b| {
                    markup::fitted(
                        &c.title,
                        b,
                        if c.truncate == tmt_cli_style::grid::Truncate::Middle {
                            TextFlow::Middle
                        } else {
                            TextFlow::Truncate
                        },
                        c.align,
                    )
                    .remove(0)
                })
            })
            .collect::<Vec<_>>()
            .join(" ");
        let header = format!("  {header}");
        let header_width = header.width().min(width);
        scene.label(header, 0, header_width, Some(Role::Muted), None);
        let mut y = 1;
        let mut at = 0;
        for item in app.items() {
            let row = match item {
                Item::Header(title) => {
                    let title = title.to_uppercase();
                    let index = scene.label(title.clone(), y, title.width().min(width), None, None);
                    scene.parts[index].emphasize = true;
                    y += 1;
                    continue;
                }
                Item::Row(row) => row,
            };
            scene.starts.push(y);
            let age = crate::staleness::label(&row["staleness"]);
            scene.ages.push(age.is_some());
            let mut root = admitted[at].clone();
            root.children.clear();
            let parent = scene.add(root, (0, y, width, 1), None);
            scene.parts[parent].row = Some(at);
            for (line, configured) in rows.lines.iter().enumerate() {
                let backdrop = scene.label(String::new(), y, 2, None, Some(parent));
                let mut x = 2;
                let mut position = 0;
                let mut parts = Vec::new();
                let mut shown = line == 0;
                let mut height = 1;
                for (cell, node) in configured.iter().zip(&admitted[at].children[line].children) {
                    let start = position;
                    position += cell.span;
                    let Some(box_width) = layout.span(start..position) else {
                        continue;
                    };
                    let value = node.text.as_deref();
                    shown |= value.is_some_and(|v| !v.is_empty());
                    let mut node = node.clone();
                    if value.is_none() {
                        node.text = Some(
                            if cell.field.is_some() && line == 0 {
                                "–"
                            } else {
                                ""
                            }
                            .into(),
                        );
                    }
                    let logical = tmt_tui::text::lines(
                        node.text.as_deref().unwrap_or_default(),
                        box_width.text,
                        node.style.text_flow,
                    )
                    .len();
                    height = height.max(logical);
                    let index = scene.add(node, (x, y, box_width.visible, 1), Some(parent));
                    let part = &mut scene.parts[index];
                    part.width = box_width.text;
                    part.cut = box_width.cut;
                    part.align = rows.columns[start].align;
                    let token = cell
                        .field
                        .as_deref()
                        .and_then(|field| row["colors"][field].as_str());
                    let failed = cell.field.as_deref().is_some_and(|field| {
                        row["failed"]
                            .as_array()
                            .is_some_and(|v| v.iter().any(|v| v == field))
                    });
                    let role = part
                        .node
                        .style
                        .token
                        .filter(|_| !failed || token.is_some())
                        .or_else(|| token.and_then(crate::look::role));
                    part.node.style.token =
                        if value.is_none_or(str::is_empty) || (token.is_none() && failed) {
                            Some(Role::Dim)
                        } else {
                            role
                        };
                    part.emphasize = value.is_some_and(|v| !v.is_empty())
                        && (matches!(cell.field.as_deref(), Some("state" | "pending"))
                            || role.is_some_and(|r| matches!(r, Role::Waiting | Role::Blocked)));
                    parts.push(index);
                    x += box_width.visible + 1;
                }
                if !shown {
                    scene.parts.truncate(backdrop);
                    continue;
                }
                scene.parts[backdrop].rect.width = x.saturating_sub(1).max(2).min(width) as u32;
                scene.parts[backdrop].rect.height = height as u32;
                for index in parts {
                    scene.parts[index].rect.height = height as u32;
                }
                for visual in 0..height {
                    let first = line == 0 && visual == 0;
                    let marker = if first && row["pending"].is_string() {
                        "◆ "
                    } else {
                        "  "
                    };
                    let index = scene.label(marker.into(), y + visual, 2, None, Some(parent));
                    scene.parts[index].emphasize = first && row["pending"].is_string();
                }
                if line == 0
                    && let Some(age) = &age
                {
                    let tail = width.saturating_sub(x.saturating_sub(1));
                    if age.width() < tail {
                        scene.parts[backdrop].rect.width = width as u32;
                        let index =
                            scene.label(age.clone(), y, age.width(), Some(Role::Dim), Some(parent));
                        scene.parts[index].rect.x = (width - age.width()) as i32;
                    }
                }
                y += height;
            }
            if let Some(text) = row["annotation"]["text"].as_str() {
                let to = row["annotation"]["to"].as_str().unwrap_or_default();
                let index = scene.label(
                    format!("    ✎ sent to {to}: {text}"),
                    y,
                    width,
                    Some(Role::Dim),
                    Some(parent),
                );
                scene.parts[index].node.style.text_flow = TextFlow::Truncate;
                scene.parts[index].row = None;
                y += 1;
            }
            scene.parts[parent].rect.height = (y - scene.starts[at]) as u32;
            scene.ends.push(y);
            at += 1;
        }
        if at == 0 {
            let message = if app.search.is_empty() {
                "  (no members)"
            } else {
                "  (no matching members)"
            };
            scene.label(
                message.into(),
                y,
                message.width().min(width),
                Some(Role::Dim),
                None,
            );
            y += 1;
        }
        scene.height = y;
        scene
    }
    pub fn paint(
        &self,
        buffer: &mut Buffer,
        body: Rect,
        offset: usize,
        selected: usize,
        look: Look,
    ) -> Vec<Hit> {
        let bounds = BoxRect {
            x: i32::from(body.x),
            y: i32::from(body.y),
            width: u32::from(body.width),
            height: u32::from(body.height),
        };
        let cells = self
            .parts
            .iter()
            .map(|part| {
                let rect = BoxRect {
                    x: part.rect.x + bounds.x,
                    y: part.rect.y + bounds.y - offset as i32,
                    ..part.rect
                };
                Cell {
                    node: &part.node,
                    parent: part.parent,
                    rect,
                    content: rect,
                    clip: rect.intersect(bounds),
                    text_width: part.width,
                    cut: part.cut,
                    overflowing: false,
                }
            })
            .collect::<Vec<_>>();
        let hits = tmt_tui::paint::paint_with(&cells, buffer, |index, _, _| {
            let part = &self.parts[index];
            if part.parent.is_none() && part.row.is_some() {
                return (Style::new(), part.align);
            }
            let selected = part.row == Some(selected);
            let base = if selected {
                look.selection()
            } else if part.row.is_some_and(|row| self.ages[row]) {
                look.role(Role::Dim)
            } else {
                Style::new()
            };
            let mut style = part
                .node
                .style
                .token
                .map_or_else(Style::new, |role| look.role(role));
            if part.row.is_none() && part.emphasize {
                style = style.add_modifier(Modifier::BOLD);
            }
            (
                base.patch(look.row_span(selected, style, part.emphasize)),
                part.align,
            )
        });
        let mut result = Vec::new();
        for (index, part) in self
            .parts
            .iter()
            .enumerate()
            .filter(|(_, p)| p.parent.is_none() && p.row.is_some())
        {
            // Anonymous display rows retain selection coverage, without inventing IDs.
            let rect = match part.node.id.as_deref() {
                Some(id) => hits
                    .iter()
                    .find(|hit| hit.id == Some(id))
                    .map(|hit| hit.rect),
                None => Some(cells[index].clip.intersect(bounds)),
            };
            let Some(rect) = rect.filter(|rect| rect.width > 0) else {
                continue;
            };
            for y in rect.y..rect.y + rect.height as i32 {
                result.push(Hit {
                    y: y as u16,
                    x: body.x,
                    width: body.width,
                    row: part.row.unwrap(),
                });
            }
        }
        result
    }
}
