//! The rows pane scene: admitted cells and solved boxes, painted by `tmt-tui`.
//! `App` owns selection and actions, `Scrolls` the position; this adapter turns
//! them into buffer cells and clipped row hits.
use crate::board::app::Hit;
use crate::display_rows::Item;
use crate::{look::Look, markup, rows::Rows};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use serde_json::Value;
use std::sync::OnceLock;
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::{
    binding::Node,
    geometry::{Cell, Rect as BoxRect},
    style::{CellStyle, TextFlow},
};
use unicode_width::UnicodeWidthStr;

/// Space between grid columns.
pub(in crate::board) const GAP: usize = 1;

/// Per-row inputs that change without a new snapshot (clock text); part of the
/// cache key, so a changed label rebuilds the scene.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::board) struct Extra {
    /// The squad's lead row: its name cell carries a dim `lead` tag.
    pub lead: bool,
    /// `cron next` for the member's cron job.
    pub next: Option<String>,
    /// Age of the oldest request waiting on the user.
    pub request_age: Option<String>,
    /// The `✓ sent` feedback line is shown under the row.
    pub sent: bool,
    /// Blank lines under the row for the inline input band.
    pub reserve: usize,
}

/// The row-end label candidates, longest first: the age mark then `cron next`, then
/// the age mark alone; `cron next` is the first to drop.
pub(in crate::board) fn row_end(age: Option<String>, next: Option<String>) -> Vec<String> {
    match (age, next) {
        (Some(age), Some(next)) => vec![format!("{age}  {next}"), age],
        (Some(age), None) => vec![age],
        (None, Some(next)) => vec![next],
        (None, None) => vec![],
    }
}

struct Part {
    node: Node,
    rect: BoxRect,
    width: u16,
    cut: bool,
    parent: Option<usize>,
    /// The row whose selection and stale styling decorate this part.
    row: Option<usize>,
    /// A row's identity root: it carries IDs and hits, never a style.
    root: bool,
    /// The leading `◆`: waiting-colored where there is color, plain in NO_COLOR.
    marker: bool,
    emphasize: bool,
    align: Align,
}

pub(in crate::board) struct RowPaint {
    parts: Vec<Part>,
    stale: Vec<bool>,
    /// First and one-past-last scene line of each row, annotation included.
    pub starts: Vec<usize>,
    pub ends: Vec<usize>,
    pub height: usize,
    /// The scene lines reserved for the inline input band, if a row has them.
    pub input: Option<std::ops::Range<usize>>,
}

fn text_style() -> CellStyle {
    static TEXT: OnceLock<CellStyle> = OnceLock::new();
    TEXT.get_or_init(|| {
        tmt_tui::parse("squad.rows", "<tmt-view version='1'><tmt-text/></tmt-view>")
            .expect("literal text prototype")
            .children
            .remove(0)
            .style
    })
    .clone()
}

impl RowPaint {
    fn add(
        &mut self,
        node: Node,
        rect: (usize, usize, usize, usize),
        parent: Option<usize>,
    ) -> usize {
        let (x, y, width, height) = rect;
        self.parts.push(Part {
            node,
            rect: BoxRect {
                x: x as i32,
                y: y as i32,
                width: width as u32,
                height: height as u32,
            },
            width: width.min(usize::from(u16::MAX)) as u16,
            cut: false,
            parent,
            row: parent.and_then(|index| self.parts[index].row),
            root: false,
            marker: false,
            emphasize: false,
            align: Align::Left,
        });
        self.parts.len() - 1
    }

    /// A one-line literal strip; `None` text is a styled blank (the row band).
    fn label(
        &mut self,
        text: Option<String>,
        rect: (usize, usize, usize, usize),
        role: Option<Role>,
        flow: TextFlow,
        parent: Option<usize>,
    ) -> usize {
        let mut style = text_style();
        style.token = role;
        style.text_flow = flow;
        let node = Node {
            kind: tmt_tui::Kind::Text,
            style,
            id: None,
            row_id: None,
            selected: false,
            text,
            children: Vec::new(),
        };
        self.add(node, rect, parent)
    }

    /// Build the scene for `items` at `width`. `cells` holds each row's admitted
    /// cells in row order and `extras` its clock-derived labels.
    pub fn build(
        rows: &Rows,
        layout: &markup::Grid,
        cells: &[Node],
        items: &[Item<'_>],
        extras: &[Extra],
        width: usize,
        empty: &str,
    ) -> Self {
        let mut scene = Self {
            parts: Vec::new(),
            stale: Vec::new(),
            starts: Vec::new(),
            ends: Vec::new(),
            height: 1,
            input: None,
        };
        let titles = rows
            .columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| {
                layout.span(index..index + 1).map(|box_width| {
                    markup::fitted(
                        &column.title,
                        box_width,
                        if column.truncate == tmt_cli_style::grid::Truncate::Middle {
                            TextFlow::Middle
                        } else {
                            TextFlow::Truncate
                        },
                        column.align,
                    )
                    .remove(0)
                })
            })
            .collect::<Vec<_>>()
            .join(&" ".repeat(GAP));
        let titles = format!("  {titles}");
        let shown = titles.width().min(width);
        scene.label(
            Some(titles),
            (0, 0, shown, 1),
            Some(Role::Muted),
            TextFlow::Clip,
            None,
        );
        let mut y = 1;
        let mut at = 0;
        for item in items {
            let row = match item {
                Item::Section(None) => continue,
                Item::Section(Some(title)) => {
                    let title = title.to_uppercase();
                    let shown = title.width().min(width);
                    let index =
                        scene.label(Some(title), (0, y, shown, 1), None, TextFlow::Clip, None);
                    scene.parts[index].emphasize = true;
                    y += 1;
                    continue;
                }
                Item::Rule(rule) => {
                    let title = format!("── {} ", rule.label());
                    let tail = width.saturating_sub(title.width());
                    let line = format!("{title}{}", "─".repeat(tail));
                    let shown = line.width().min(width);
                    scene.label(
                        Some(line),
                        (0, y, shown, 1),
                        Some(Role::Dim),
                        TextFlow::Clip,
                        None,
                    );
                    y += 1;
                    continue;
                }
                Item::Row(_, row) => row,
            };
            y = scene.row(rows, layout, &cells[at], row, &extras[at], at, y, width);
            at += 1;
        }
        if at == 0 {
            scene.label(
                Some(empty.to_owned()),
                (0, y, empty.width().min(width), 1),
                Some(Role::Dim),
                TextFlow::Clip,
                None,
            );
            y += 1;
        }
        scene.height = y;
        scene
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        &mut self,
        rows: &Rows,
        layout: &markup::Grid,
        admitted: &Node,
        row: &Value,
        extra: &Extra,
        at: usize,
        start: usize,
        width: usize,
    ) -> usize {
        let mut y = start;
        self.starts.push(y);
        let age = crate::staleness::label(&row["staleness"]);
        self.stale.push(age.is_some());
        let waits = crate::attention::waits_on_you(row);
        let labels = row_end(age, extra.next.clone());
        let mut identity = admitted.clone();
        identity.children.clear();
        let root = self.add(identity, (0, y, width, 1), None);
        self.parts[root].root = true;
        self.parts[root].row = Some(at);
        for (line, configured) in rows.lines.iter().enumerate() {
            let first = line == 0;
            let mut x = 2;
            let mut position = 0;
            let mark = self.parts.len();
            // The row's selection and stale styling reach only as far as its text
            // and row-end label do, so each line has its own backdrop.
            let backdrop = self.label(None, (0, y, 2, 1), None, TextFlow::Clip, Some(root));
            let mut shown_any = false;
            let mut height = None::<usize>;
            for (cell, node) in configured.iter().zip(&admitted.children[line].children) {
                let range = position..position + cell.span;
                position += cell.span;
                let Some(box_width) = layout.span(range.clone()) else {
                    continue;
                };
                let pending = cell.field.as_deref() == Some("pending");
                let value = if pending {
                    super::waiting::text(row)
                } else {
                    node.text.as_deref()
                };
                shown_any |= value.is_some_and(|value| !value.is_empty());
                let text = match (value, &cell.field, first) {
                    (Some(value), _, _) => value.to_owned(),
                    (None, Some(_), true) => "–".to_owned(),
                    _ => String::new(),
                };
                let column = &rows.columns[range.start];
                let mut node = node.clone();
                let (text_width, cut, count) = if pending && !first {
                    // The decision line carries its request age at the cell's right edge.
                    let age = extra.request_age.as_deref();
                    let age_width = age.map_or(0, |age| age.width() + GAP);
                    let question = super::fit(&text, box_width.visible.saturating_sub(age_width));
                    node.style.text_flow = TextFlow::Clip;
                    node.text = Some(match age {
                        Some(age) => format!("{question} {age}"),
                        None => question,
                    });
                    (
                        box_width.visible.min(usize::from(u16::MAX)) as u16,
                        false,
                        1,
                    )
                } else {
                    let count = tmt_tui::text::fit_lines(
                        &text,
                        box_width.text,
                        node.style.text_flow,
                        column.align,
                    )
                    .len();
                    node.text = Some(text);
                    (box_width.text, box_width.cut, count)
                };
                height = Some(height.map_or(count, |height| height.max(count)));
                // Admitted cell tokens override projected decoration; absent and
                // failed providers without projected colors stay quiet.
                let failed = cell.field.as_deref().is_some_and(|field| {
                    row["failed"]
                        .as_array()
                        .is_some_and(|failed| failed.iter().any(|name| name == field))
                });
                let token = cell
                    .field
                    .as_deref()
                    .and_then(|field| row["colors"][field].as_str());
                let role = node
                    .style
                    .token
                    .filter(|_| !failed || token.is_some())
                    .or_else(|| token.and_then(crate::look::role));
                let nothing = value.is_none_or(str::is_empty);
                node.style.token = if nothing || (failed && token.is_none()) {
                    Some(Role::Dim)
                } else {
                    role
                };
                let index = self.add(node, (x, y, box_width.visible, 1), Some(root));
                let part = &mut self.parts[index];
                part.width = text_width;
                part.cut = cut;
                part.align = column.align;
                part.emphasize = !nothing
                    && (matches!(cell.field.as_deref(), Some("state" | "pending"))
                        || role.is_some_and(|role| matches!(role, Role::Waiting | Role::Blocked)));
                if extra.lead && first && cell.field.as_deref() == Some("member") {
                    let name = self.parts[index].node.text.clone().unwrap_or_default();
                    self.lead_tag(&name, &box_width, column, x, y, root);
                }
                x += box_width.visible + GAP;
            }
            if !first && !shown_any {
                // A later line with nothing to show is left out.
                self.parts.truncate(mark);
                continue;
            }
            let height = height.unwrap_or(1);
            let used = (x - GAP).max(2);
            self.parts[backdrop].rect.width = used.min(width) as u32;
            for part in &mut self.parts[mark..] {
                part.rect.height = height as u32;
            }
            for visual in 0..height {
                let initial = first && visual == 0;
                let mark = initial && waits;
                // Only the diamond takes the token; its blank follows the row's style.
                let marker = self.label(
                    Some(if mark { "◆" } else { "  " }.into()),
                    (0, y + visual, if mark { 1 } else { 2 }, 1),
                    mark.then_some(Role::Waiting),
                    TextFlow::Clip,
                    Some(root),
                );
                self.parts[marker].marker = mark;
                self.parts[marker].emphasize = mark;
            }
            if first && height > 0 {
                // Its age mark: the first candidate that fits after the cells.
                self.row_end(root, &labels, used, y, width);
            }
            y += height;
            if first
                && !rows
                    .lines
                    .iter()
                    .flatten()
                    .any(|cell| cell.field.as_deref() == Some("pending"))
                && let Some(question) = super::waiting::text(row)
            {
                let age_width = extra
                    .request_age
                    .as_deref()
                    .map_or(0, |age| age.width() + GAP);
                let room = width.saturating_sub(4 + age_width);
                self.label(
                    None,
                    (0, y, (4 + room).min(width), 1),
                    None,
                    TextFlow::Clip,
                    Some(root),
                );
                let index = self.label(
                    Some(question.to_owned()),
                    (4, y, room, 1),
                    Some(Role::Waiting),
                    TextFlow::Truncate,
                    Some(root),
                );
                self.parts[index].emphasize = true;
                if let Some(age) = &extra.request_age {
                    self.row_end(root, std::slice::from_ref(age), 4 + room, y, width);
                }
                y += 1;
            }
        }
        if extra.sent {
            let text = "    ✓ sent";
            let index = self.label(
                Some(text.into()),
                (0, y, text.width().min(width), 1),
                Some(Role::Working),
                TextFlow::Clip,
                Some(root),
            );
            self.parts[index].row = None;
            y += 1;
        }
        if let Some(text) = row["annotation"]["text"].as_str() {
            let to = row["annotation"]["to"].as_str().unwrap_or_default();
            self.label(
                Some(format!("    ✎ sent to {to}: {text}")),
                (0, y, width, 1),
                Some(Role::Dim),
                TextFlow::Truncate,
                Some(root),
            );
            // Decoration is independent of the row: selection never reaches it.
            let index = self.parts.len() - 1;
            self.parts[index].row = None;
            y += 1;
        }
        // The row's identity, and so its clicks, stop before the reserved input lines.
        self.parts[root].rect.height = (y - start) as u32;
        if extra.reserve > 0 {
            self.input = Some(y..y + extra.reserve);
            y += extra.reserve;
        }
        self.ends.push(y);
        y
    }

    /// The dim `lead` tag two cells after the name, inside the name cell: it
    /// reserves no width on other rows and is the first to be cut when the cell
    /// is narrow, so the name keeps its room.
    fn lead_tag(
        &mut self,
        name: &str,
        cell: &markup::BoxWidth,
        column: &crate::rows::Column,
        x: usize,
        y: usize,
        root: usize,
    ) {
        let shown = tmt_tui::text::fit_lines(name, cell.text, markup::flow(column), column.align)
            .into_iter()
            .next()
            .map_or(0, |line| line.trim_end().width());
        let from = shown + 2;
        let room = cell.visible.saturating_sub(from).min("lead".len());
        // A lone ellipsis says nothing: below two cells the tag is cut entirely.
        if room < 2 {
            return;
        }
        self.label(
            Some("lead".into()),
            (x + from, y, room, 1),
            Some(Role::Dim),
            TextFlow::Truncate,
            Some(root),
        );
    }

    /// Right-align the first label that fits after `used` cells of the line.
    fn row_end(&mut self, root: usize, labels: &[String], used: usize, y: usize, width: usize) {
        let Some(label) = labels
            .iter()
            .find(|label| used + GAP + label.width() <= width)
        else {
            return;
        };
        let at = width - label.width();
        // The gap before the label keeps the row's base style.
        self.label(
            None,
            (used, y, width - used, 1),
            None,
            TextFlow::Clip,
            Some(root),
        );
        self.label(
            Some(label.clone()),
            (at, y, label.width(), 1),
            Some(Role::Dim),
            TextFlow::Clip,
            Some(root),
        );
    }

    /// Paint into `body`, `offset` lines down the scene, and return one hit per
    /// visible row line.
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
        let painted = tmt_tui::paint::paint_with(&cells, buffer, |index, _, _| {
            let part = &self.parts[index];
            if part.root {
                return (Style::new(), Align::Left);
            }
            let selected = part.row == Some(selected);
            let base = if selected {
                look.selection()
            } else if part.row.is_some_and(|row| self.stale[row]) {
                look.role(Role::Dim)
            } else {
                Style::new()
            };
            // Without color the mark keeps its plain style.
            let plain = part.marker && look.depth == tmt_cli_style::Depth::None;
            let mut style = part
                .node
                .style
                .token
                .filter(|_| !plain)
                .map_or_else(Style::new, |role| look.role(role));
            if part.row.is_none() && part.emphasize {
                style = style.add_modifier(Modifier::BOLD);
            }
            (
                base.patch(look.row_span(selected, style, part.emphasize)),
                part.align,
            )
        });
        let mut hits = Vec::new();
        for (index, part) in self.parts.iter().enumerate().filter(|(_, part)| part.root) {
            // Anonymous display rows keep click coverage without inventing IDs.
            let rect = match part.node.id.as_deref() {
                Some(id) => painted
                    .iter()
                    .find(|hit| hit.id == Some(id))
                    .map(|hit| hit.rect),
                None => Some(cells[index].clip),
            };
            let Some(rect) = rect.filter(|rect| rect.width > 0) else {
                continue;
            };
            for y in rect.y..rect.y + rect.height as i32 {
                hits.push(Hit {
                    y: y as u16,
                    x: body.x,
                    width: body.width,
                    row: part.row.expect("a root names its row"),
                });
            }
        }
        hits
    }
}
