//! HOME sections as admitted markup. A section is a literal template bound to
//! display-ready data, solved by `tmt-tui` geometry at the body width and painted
//! into a scratch buffer whose rows join HOME's line stream. The stream, the
//! shared cursor, hits, reveal and input reservations stay with the painter that
//! owns them; a section only reports where each of its blocks landed.
use crate::{board::picker_surface::Data, look::Look};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
};
use serde_json::Value;
use std::ops::Range;
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::{
    binding::{Schema, Template},
    geometry::{self, Rect as BoxRect},
    paint, text,
};
use unicode_width::UnicodeWidthStr;

/// A compiled section template. Markup and schema are literals of the section,
/// so a failure is a programming error, never a data error.
pub(super) fn compile(file: &str, markup: &str, schema: &Schema) -> Template<()> {
    let parsed = tmt_tui::parse(file, markup).expect("embedded HOME markup");
    tmt_tui::binding::compile(file, &parsed, schema, &Data).expect("embedded HOME schema")
}

/// Everything a section's painted output is a function of: the bound data (every
/// clock-derived label, composer reservation and feedback line enters through
/// it), the width, the look and the selected block. A section's template is fixed
/// per slot, so equal keys paint equal lines.
#[derive(PartialEq)]
pub(super) struct Key {
    pub width: u16,
    pub look: Look,
    pub selected: Option<String>,
    pub data: Value,
}

/// A section's last painted output with the key it was painted for. It lives in
/// the immutable view's derivations, so a new snapshot starts empty.
pub(super) struct Kept<T> {
    held: Option<(Key, T)>,
    #[cfg(test)]
    pub builds: usize,
}

impl<T> Default for Kept<T> {
    fn default() -> Self {
        Self {
            held: None,
            #[cfg(test)]
            builds: 0,
        }
    }
}

impl<T> Kept<T> {
    /// The output for `key`: the held one when the key is unchanged, otherwise
    /// `build`'s, which replaces it.
    pub fn get(&mut self, key: Key, build: impl FnOnce(&Key) -> T) -> &T {
        if self.held.as_ref().is_none_or(|(held, _)| *held != key) {
            let value = build(&key);
            self.held = Some((key, value));
            #[cfg(test)]
            {
                self.builds += 1;
            }
        }
        &self.held.as_ref().expect("a held scene").1
    }
}

/// The painted section and where each identified node landed.
pub(super) struct Painted {
    pub lines: Vec<Line<'static>>,
    nodes: Vec<(Vec<String>, BoxRect)>,
}

impl Painted {
    fn rect(&self, id: &[&str]) -> Option<&BoxRect> {
        self.nodes
            .iter()
            .find(|(own, _)| own.iter().map(String::as_str).eq(id.iter().copied()))
            .map(|(_, rect)| rect)
    }

    /// The scene lines a node covers.
    pub fn lines(&self, id: &[&str]) -> Option<Range<usize>> {
        self.rect(id)
            .map(|rect| rect.y as usize..(rect.y as u32 + rect.height) as usize)
    }
}

/// The node being decorated: its own scoped identity (the last element names the
/// part), the identity it inherits from the nearest ancestor that has one (the
/// block it belongs to), and its inherited role.
pub(super) struct Part<'a> {
    pub id: Option<&'a [String]>,
    pub scope: Option<&'a [String]>,
    pub role: Role,
}

/// The complete style of a node and its alignment inside the recorded width.
pub(super) type Decorate<'a> = dyn FnMut(Part<'_>) -> (Style, Align) + 'a;

/// Solve and paint `template` bound to `data` at `width` cells.
pub(super) fn paint(
    file: &str,
    template: &Template<()>,
    data: &Value,
    width: u16,
    decorate: &mut Decorate<'_>,
) -> Painted {
    let root = template
        .materialize(file, data, &Data)
        .unwrap_or_else(|error| panic!("HOME projection: {error}"));
    let cells = geometry::layout(&root, [width, u16::MAX], text::measure)
        .unwrap_or_else(|error| panic!("HOME geometry: {error}"));
    // The root is as tall as the probe viewport; the content is what its children cover.
    let height = cells
        .iter()
        .skip(1)
        .map(|cell| cell.rect.y.max(0) as u32 + cell.rect.height)
        .max()
        .unwrap_or(0)
        .min(u32::from(u16::MAX)) as u16;
    // A root without content has no rows and no cells to lift.
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
    paint::paint_with(&cells, &mut buffer, |index, role, _| {
        let scope = std::iter::successors(Some(index), |at| cells[*at].parent)
            .find_map(|at| cells[at].node.id.as_deref());
        decorate(Part {
            id: cells[index].node.id.as_deref(),
            scope,
            role,
        })
    });
    let nodes = cells
        .iter()
        .filter_map(|cell| cell.node.id.clone().map(|id| (id, cell.rect)))
        .collect();
    Painted {
        lines: lift(&buffer),
        nodes,
    }
}

/// A buffer row as a line of spans: neighbouring cells of one style join, and the
/// continuation cell of a wide grapheme is the grapheme's, not a space of its own.
fn lift(buffer: &Buffer) -> Vec<Line<'static>> {
    let width = usize::from(buffer.area.width);
    if width == 0 {
        return vec![Line::default(); usize::from(buffer.area.height)];
    }
    buffer
        .content
        .chunks(width)
        .map(|cells| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut skip = 0;
            let mut last: Option<Style> = None;
            for cell in cells {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                skip = cell.symbol().width().saturating_sub(1);
                let style = cell.style();
                match (spans.last_mut(), last) {
                    (Some(span), Some(previous)) if previous == style => {
                        span.content.to_mut().push_str(cell.symbol());
                    }
                    _ => spans.push(Span::styled(cell.symbol().to_owned(), style)),
                }
                last = Some(style);
            }
            Line::from(spans)
        })
        .collect()
}
