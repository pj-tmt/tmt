//! One Taffy computation; no application priority policy, painting or terminal owner.
use crate::{MAX_DEPTH, MAX_NODES, binding::Node, style::*};
use taffy::{geometry as tg, prelude as t};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Rect {
    pub fn intersect(self, other: Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (i64::from(self.x) + i64::from(self.width))
            .min(i64::from(other.x) + i64::from(other.width));
        let bottom = (i64::from(self.y) + i64::from(self.height))
            .min(i64::from(other.y) + i64::from(other.height));
        Self {
            x,
            y,
            width: (right - i64::from(x)).max(0) as u32,
            height: (bottom - i64::from(y)).max(0) as u32,
        }
    }
}

/// Cell constraint delivered to the scalar text owner's single measurer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Space {
    Cells(u16),
    MinContent,
    MaxContent,
}

/// Borrowed identity and style remain owned by the materialized scene.
#[derive(Debug)]
pub struct Cell<'a> {
    pub node: &'a Node,
    pub rect: Rect,
    pub content: Rect,
    pub clip: Rect,
    /// The whole-cell width given to the shared text measurer and later painter.
    /// Fractional spare cells are styled blanks inside the hit area; alignment
    /// and wrapping use this width, never the rounded spare cell.
    pub text_width: u16,
    pub cut: bool,
    pub overflowing: bool,
}
fn dimension(value: Extent) -> t::Dimension {
    match value {
        Extent::Auto => t::Dimension::Auto,
        Extent::Cells(n) => t::Dimension::Length(f32::from(n)),
        Extent::Full => t::Dimension::Percent(1.0),
        Extent::Percent(n) => t::Dimension::Percent(f32::from(n) / 100.0),
    }
}
fn track(value: &GridTrack) -> t::TrackSizingFunction {
    use taffy::style::{MaxTrackSizingFunction as Max, MinTrackSizingFunction as Min};
    let min = match value.min {
        Breadth::Auto => Min::Auto,
        Breadth::Cells(n) => Min::Fixed(t::LengthPercentage::Length(f32::from(n))),
        Breadth::Percent(n) => Min::Fixed(t::LengthPercentage::Percent(f32::from(n) / 100.0)),
        Breadth::Fraction(_) => unreachable!("admitted minmax minimum"),
    };
    let max = match value.max {
        Breadth::Auto => Max::Auto,
        Breadth::Cells(n) => Max::Fixed(t::LengthPercentage::Length(f32::from(n))),
        Breadth::Percent(n) => Max::Fixed(t::LengthPercentage::Percent(f32::from(n) / 100.0)),
        Breadth::Fraction(n) => Max::Fraction(f32::from(n)),
    };
    t::TrackSizingFunction::Single(tg::MinMax { min, max })
}
fn style(node: &Node) -> t::Style {
    let s = &node.style;
    t::Style {
        display: if s.display == Display::Grid {
            t::Display::Grid
        } else {
            t::Display::Flex
        },
        flex_direction: if s.direction == Direction::Row {
            t::FlexDirection::Row
        } else {
            t::FlexDirection::Column
        },
        size: tg::Size {
            width: dimension(s.width),
            height: dimension(s.height),
        },
        min_size: tg::Size {
            width: s
                .min_width
                .map_or(t::Dimension::Auto, |n| dimension(Extent::Cells(n))),
            height: t::Dimension::Auto,
        },
        max_size: tg::Size {
            width: s
                .max_width
                .map_or(t::Dimension::Auto, |n| dimension(Extent::Cells(n))),
            height: t::Dimension::Auto,
        },
        flex_basis: dimension(s.basis),
        flex_grow: f32::from(s.grow),
        flex_shrink: f32::from(s.shrink),
        gap: tg::Size {
            width: t::LengthPercentage::Length(f32::from(s.gap[0])),
            height: t::LengthPercentage::Length(f32::from(s.gap[1])),
        },
        padding: tg::Rect {
            left: t::LengthPercentage::Length(f32::from(s.padding[0])),
            right: t::LengthPercentage::Length(f32::from(s.padding[0])),
            top: t::LengthPercentage::Length(f32::from(s.padding[1])),
            bottom: t::LengthPercentage::Length(f32::from(s.padding[1])),
        },
        overflow: tg::Point {
            x: taffy::style::Overflow::Hidden,
            y: taffy::style::Overflow::Hidden,
        },
        grid_template_columns: s.columns.iter().map(track).collect(),
        grid_column: tg::Line {
            start: t::GridPlacement::Auto,
            end: t::GridPlacement::Span(s.col_span),
        },
        ..Default::default()
    }
}
// Flat preorder mapping, not another layout tree or solver.
type GeometryEntry<'a> = (t::NodeId, &'a Node, Option<usize>);
fn insert<'a>(
    tree: &mut t::TaffyTree<&'a Node>,
    entries: &mut Vec<GeometryEntry<'a>>,
    node: &'a Node,
    parent: Option<usize>,
    depth: usize,
) -> Result<t::NodeId, String> {
    if depth >= MAX_DEPTH || entries.len() >= MAX_NODES as usize {
        return Err("geometry exceeds scene depth/node limit".into());
    }
    if node.style.col_span == 0
        || node
            .style
            .columns
            .iter()
            .any(|track| matches!(track.min, Breadth::Fraction(_)))
    {
        return Err("inadmissible grid span/minmax minimum".into());
    }
    let id = tree
        .new_leaf_with_context(style(node), node)
        .map_err(|e| e.to_string())?;
    let index = entries.len();
    entries.push((id, node, parent));
    let children = node
        .children
        .iter()
        .map(|child| insert(tree, entries, child, Some(index), depth + 1))
        .collect::<Result<Vec<_>, _>>()?;
    tree.set_children(id, &children)
        .map_err(|e| e.to_string())?;
    Ok(id)
}
fn budget(width: f32) -> u16 {
    width.floor().clamp(0.0, f32::from(u16::MAX)) as u16
}

/// `measure_text` owns intrinsic widths and bounded wrapping/clamp together.
/// It must use exactly the supplied width again at paint time; no second wrapper.
/// Markup percentages are CSS content-box shares; gaps are additional space.
/// Squad chooses visible tracks before calling this API (#774), never here.
pub fn layout(
    root: &Node,
    viewport: [u16; 2],
    mut measure_text: impl FnMut(&str, TextFlow, Space) -> [u16; 2],
) -> Result<Vec<Cell<'_>>, String> {
    let mut tree = t::TaffyTree::new();
    let mut entries = Vec::new();
    let id = insert(&mut tree, &mut entries, root, None, 0)?;
    let mut root_style = tree.style(id).map_err(|e| e.to_string())?.clone();
    if root.style.width == Extent::Auto {
        root_style.size.width = dimension(Extent::Cells(viewport[0]));
    }
    if root.style.height == Extent::Auto {
        root_style.size.height = dimension(Extent::Cells(viewport[1]));
    }
    tree.set_style(id, root_style).map_err(|e| e.to_string())?;
    tree.compute_layout_with_measure(
        id,
        tg::Size {
            width: t::AvailableSpace::Definite(f32::from(viewport[0])),
            height: t::AvailableSpace::Definite(f32::from(viewport[1])),
        },
        |_, available, _, context, _| {
            let Some(text_node) = context else {
                return tg::Size::ZERO;
            };
            let Some(text) = text_node.text.as_deref() else {
                return tg::Size::ZERO;
            };
            // Taffy already subtracts padding from the available content width.
            let space = match available.width {
                t::AvailableSpace::Definite(n) => Space::Cells(budget(n)),
                t::AvailableSpace::MinContent => Space::MinContent,
                t::AvailableSpace::MaxContent => Space::MaxContent,
            };
            let [width, height] = measure_text(text, text_node.style.text_flow, space);
            tg::Size {
                width: f32::from(width),
                height: f32::from(height),
            }
        },
    )
    .map_err(|e| e.to_string())?;
    let viewport = Rect {
        width: u32::from(viewport[0]),
        height: u32::from(viewport[1]),
        ..Default::default()
    };
    let mut result: Vec<Cell<'_>> = Vec::with_capacity(entries.len());
    let mut origins = Vec::with_capacity(entries.len());
    for (id, node, parent) in entries {
        let raw = tree.unrounded_layout(id);
        let rounded = tree.layout(id).map_err(|e| e.to_string())?;
        let [x, y] = parent.map_or([0.0, 0.0], |index| origins[index]);
        let origin = [x + raw.location.x, y + raw.location.y];
        origins.push(origin);
        // Absolute edges use Taffy's cumulative rounding, avoiding nested gaps.
        let rect = Rect {
            x: origin[0].round() as i32,
            y: origin[1].round() as i32,
            width: rounded.size.width as u32,
            height: rounded.size.height as u32,
        };
        let px = u32::from(node.style.padding[0]);
        let py = u32::from(node.style.padding[1]);
        let content = Rect {
            x: rect.x.saturating_add(px as i32),
            y: rect.y.saturating_add(py as i32),
            width: rect.width.saturating_sub(2 * px),
            height: rect.height.saturating_sub(2 * py),
        };
        let ancestor = parent.map_or(viewport, |index| {
            result[index].content.intersect(result[index].clip)
        });
        let mut clip = rect.intersect(ancestor);
        let cut = parent.is_some_and(|index| result[index].node.style.display == Display::Grid)
            && clip.width > 0
            && clip.width < rect.width;
        if cut && clip.width < 4 {
            clip.width = 0;
            clip.height = 0;
        }
        result.push(Cell {
            node,
            rect,
            content,
            clip,
            text_width: budget(raw.size.width - 2.0 * px as f32)
                .min(content.width.min(u32::from(u16::MAX)) as u16),
            cut,
            overflowing: rounded.content_size.width > rounded.size.width
                || rounded.content_size.height > rounded.size.height,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
