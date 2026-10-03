//! Embedded composition admission; named rich painters retain their owners.
use crate::{
    config::{Board, BoardMode, Direction, Pane},
    split::{Size, Split},
};
use ratatui::layout::Rect;
use std::collections::BTreeSet;
use tmt_tui::{
    binding::{self, Node, Schema, Schemas, Scopes, Sources},
    geometry,
    style::Extent,
};

const FILE: &str = "squad.composition.xml";
const SCAFFOLD: &str = "<tmt-view version='1' class='flex-col'><tmt-row class='min-w-0 shrink-0'/><tmt-col class='min-w-0 shrink-0'/><tmt-cell class='min-w-0 shrink-0'/></tmt-view>";
struct PaneSlots;
impl Sources for PaneSlots {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("pane slots have no field sources".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("pane slots have no field sources".into())
    }
}

fn footprint(split: &Split, collapsed: &BTreeSet<Pane>) -> Option<(u16, u16)> {
    match split {
        Split::Pane(pane) => collapsed
            .contains(pane)
            .then(|| (2 + pane.title().len() as u16, 1)),
        Split::Group {
            direction,
            children,
        } => {
            let sizes = children
                .iter()
                .map(|(_, child)| footprint(child, collapsed))
                .collect::<Option<Vec<_>>>()?;
            Some(if *direction == Direction::LeftRight {
                (
                    sizes.iter().map(|(w, _)| *w).sum::<u16>()
                        + sizes.len().saturating_sub(1) as u16,
                    sizes.iter().map(|(_, h)| *h).max().unwrap_or(0),
                )
            } else {
                (
                    sizes.iter().map(|(w, _)| *w).max().unwrap_or(0),
                    sizes.iter().map(|(_, h)| *h).sum(),
                )
            })
        }
    }
}

struct Scaffold {
    root: Node,
    row: Node,
    column: Node,
    slot: Node,
}
impl Scaffold {
    fn read() -> Result<Self, String> {
        let parsed = tmt_tui::parse(FILE, SCAFFOLD).map_err(|e| e.to_string())?;
        let schema = Schema::Object(Default::default());
        let mut root = binding::compile(FILE, &parsed, &schema, &PaneSlots)
            .and_then(|template| template.materialize(FILE, &serde_json::json!({}), &PaneSlots))
            .map_err(|e| e.to_string())?;
        let [row, column, slot]: [Node; 3] = std::mem::take(&mut root.children)
            .try_into()
            .map_err(|_: Vec<Node>| format!("{FILE}: expected row, column and pane prototypes"))?;
        Ok(Self {
            root,
            row,
            column,
            slot,
        })
    }
    fn node(&self, split: &Split) -> Node {
        match split {
            Split::Pane(_) => self.slot.clone(),
            Split::Group {
                direction: Direction::LeftRight,
                ..
            } => self.row.clone(),
            Split::Group { .. } => self.column.clone(),
        }
    }
}

fn element(split: &Split, mut node: Node, collapsed: &BTreeSet<Pane>, scaffold: &Scaffold) -> Node {
    match split {
        Split::Pane(pane) => {
            node.id = Some(vec![pane.title().into()]);
            if collapsed.contains(pane) {
                node.style.height = Extent::Cells(1);
            }
        }
        Split::Group {
            direction,
            children,
        } => {
            let horizontal = *direction == Direction::LeftRight;
            if let Some((w, h)) = footprint(split, collapsed) {
                node.style.width = Extent::Cells(w);
                node.style.height = Extent::Cells(h);
            }
            let folded: Vec<_> = children
                .iter()
                .map(|(_, child)| footprint(child, collapsed))
                .collect();
            let has_fold = folded.iter().any(Option::is_some);
            let percent: u16 = children
                .iter()
                .map(|(size, _)| if let Size::Percent(p) = size { *p } else { 0 })
                .sum();
            let grow: u16 = children
                .iter()
                .map(|(size, _)| if let Size::Grow(g) = size { *g } else { 0 })
                .sum();
            // Derived relative weights, not new authored utility values. Validated
            // config has <=4 panes and shares <=100: each product fits u16.
            let weights: Vec<u16> = children
                .iter()
                .zip(&folded)
                .map(|((size, _), fold)| {
                    if fold.is_some() {
                        0
                    } else {
                        match size {
                            Size::Percent(p) => p * grow.max(1),
                            Size::Grow(g) => (100 - percent) * g,
                        }
                    }
                })
                .collect();
            let zero = weights.iter().all(|w| *w == 0);
            for (index, ((_, child), fold)) in children.iter().zip(folded).enumerate() {
                let mut next = scaffold.node(child);
                if let Some((w, h)) = fold {
                    let gap = horizontal && index > 0;
                    next.style.basis = Extent::Cells(if horizontal { w } else { h });
                    if gap {
                        let mut wrapper = scaffold.row.clone();
                        wrapper.style.basis = Extent::Cells(w + 1);
                        let mut spacer = scaffold.slot.clone();
                        spacer.style.width = Extent::Cells(1);
                        next.style.basis = Extent::Auto;
                        next.style.grow = 1;
                        wrapper.children = vec![spacer, element(child, next, collapsed, scaffold)];
                        node.children.push(wrapper);
                        continue;
                    }
                } else {
                    match children[index].0 {
                        Size::Percent(p) if !has_fold => {
                            next.style.basis = Extent::Percent(p as u8)
                        }
                        size => {
                            next.style.basis = Extent::Cells(0);
                            next.style.grow = if has_fold {
                                if zero { 1 } else { weights[index] }
                            } else {
                                match size {
                                    Size::Grow(g) => g,
                                    _ => unreachable!("percent handled above"),
                                }
                            };
                        }
                    }
                }
                node.children
                    .push(element(child, next, collapsed, scaffold));
            }
        }
    }
    node
}

fn compile(split: &Split, collapsed: &BTreeSet<Pane>) -> Result<Node, String> {
    let scaffold = Scaffold::read()?;
    let mut child = scaffold.node(split);
    if footprint(split, collapsed).is_none() {
        child.style.grow = 1;
    }
    let child = element(split, child, collapsed, &scaffold);
    let mut root = scaffold.root;
    root.children = vec![child];
    Ok(root)
}

pub(super) fn slots(node: &Node, area: Rect) -> Result<Vec<(Vec<String>, Rect)>, String> {
    let cells = geometry::layout(node, [area.width, area.height], |_, _, _| [0, 0])?;
    cells
        .iter()
        .filter_map(|cell| cell.node.id.as_ref().map(|id| (id, cell)))
        .map(|(id, cell)| {
            let rect = cell.clip;
            // Empty intersections keep their raw start; hit coordinates must
            // remain bounded by every ancestor, including a clipped folded group.
            let (mut right, mut bottom) = (i32::from(area.width), i32::from(area.height));
            let mut parent = cell.parent;
            while let Some(index) = parent {
                let ancestor = &cells[index];
                right = right.min(ancestor.clip.x + ancestor.clip.width as i32);
                bottom = bottom.min(ancestor.clip.y + ancestor.clip.height as i32);
                parent = ancestor.parent;
            }
            Ok((
                id.clone(),
                Rect::new(
                    area.x.saturating_add(rect.x.clamp(0, right) as u16),
                    area.y.saturating_add(rect.y.clamp(0, bottom) as u16),
                    rect.width as u16,
                    rect.height as u16,
                ),
            ))
        })
        .collect()
}

fn tabs(focused: Pane) -> Result<Node, String> {
    let scaffold = Scaffold::read()?;
    let mut bar = scaffold.slot.clone();
    bar.id = Some(vec!["pane-tabs".into()]);
    bar.style.height = Extent::Cells(1);
    bar.style.shrink = 1;
    let mut pane = scaffold.slot;
    pane.id = Some(vec![focused.title().into()]);
    pane.style.height = Extent::Cells(1);
    pane.style.grow = 1;
    let mut root = scaffold.root;
    root.children = vec![bar, pane];
    Ok(root)
}

pub(super) fn admit() -> Result<(), String> {
    Scaffold::read().map(|_| ())
}

pub(super) struct Cache {
    key: (Board, BTreeSet<Pane>, Option<Pane>, Rect),
    slots: Vec<(Vec<String>, Rect)>,
}
/// Immutable views retain geometry; only inputs that affect placement invalidate it.
pub(super) fn layout(
    cache: &mut Option<Cache>,
    board: &Board,
    collapsed: &BTreeSet<Pane>,
    focused: Pane,
    area: Rect,
) -> Result<Vec<(Vec<String>, Rect)>, String> {
    let key = (
        board.clone(),
        collapsed.clone(),
        (board.mode == BoardMode::Tabs).then_some(focused),
        area,
    );
    if cache.as_ref().is_none_or(|old| old.key != key) {
        let node = if board.mode == BoardMode::Tabs {
            tabs(focused)?
        } else {
            compile(&board.split, collapsed)?
        };
        *cache = Some(Cache {
            key,
            slots: slots(&node, area)?,
        });
    }
    Ok(cache
        .as_ref()
        .expect("successful layout installed cache")
        .slots
        .clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    fn rectangles(node: &Node, area: Rect) -> Result<Vec<(Pane, Rect)>, String> {
        slots(node, area)?
            .into_iter()
            .map(|(id, rect)| Ok((Pane::parse(id.last().unwrap()).ok_or("unknown pane")?, rect)))
            .collect()
    }
    fn split(text: &str) -> Split {
        let doc: toml_edit::DocumentMut = format!("layout = {text}").parse().unwrap();
        crate::split::read(&doc["layout"], "board.layout").unwrap()
    }
    #[test]
    fn tabs_reservation_matches_literal_and_existing_geometry() {
        use ratatui::layout::{Constraint, Layout};
        admit().unwrap();
        let node = tabs(Pane::Notes).unwrap();
        assert_eq!(
            slots(&node, Rect::new(2, 3, 80, 1)).unwrap(),
            vec![
                (vec!["pane-tabs".into()], Rect::new(2, 3, 80, 0)),
                (vec!["notes".into()], Rect::new(2, 3, 80, 1)),
            ]
        );
        for w in [0, 3, 80, 120] {
            for h in 0..=32 {
                let area = Rect::new(2, 3, w, h);
                let [bar, pane] =
                    Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
                assert_eq!(
                    slots(&node, area).unwrap(),
                    vec![
                        (vec!["pane-tabs".into()], bar),
                        (vec!["notes".into()], pane)
                    ]
                );
            }
        }
    }
    #[test]
    fn folds_retain_shares_and_cache_tracks_geometry_inputs() {
        for direction in [Direction::LeftRight, Direction::TopBottom] {
            let tree = Split::simple(
                direction,
                &[Pane::Rows, Pane::Detail, Pane::Notes],
                &[40, 20, 40],
            );
            let node = compile(&tree, &BTreeSet::from([Pane::Detail])).unwrap();
            let areas = rectangles(&node, Rect::new(0, 0, 100, 101)).unwrap();
            let extent = |r: Rect| {
                if direction == Direction::LeftRight {
                    r.width
                } else {
                    r.height
                }
            };
            assert!(extent(areas[0].1).abs_diff(extent(areas[2].1)) <= 1);
            assert_eq!(
                extent(areas[1].1),
                if direction == Direction::LeftRight {
                    8
                } else {
                    1
                }
            );
            assert_eq!(areas[1].1.height, 1);
            if direction == Direction::LeftRight {
                assert_eq!(areas[1].1.x, areas[0].1.right() + 1);
            }
        }
        let mut board = Board::simple(
            BoardMode::Split,
            Direction::LeftRight,
            vec![Pane::Rows, Pane::Notes],
            &[60, 40],
        );
        let mut cache = None;
        let area = Rect::new(0, 0, 100, 20);
        let first = layout(&mut cache, &board, &BTreeSet::new(), Pane::Rows, area).unwrap();
        assert_eq!(
            layout(&mut cache, &board, &BTreeSet::new(), Pane::Notes, area).unwrap(),
            first
        );
        assert!(cache.as_ref().unwrap().key.2.is_none());
        assert_ne!(
            layout(
                &mut cache,
                &board,
                &BTreeSet::from([Pane::Notes]),
                Pane::Rows,
                area
            )
            .unwrap(),
            first
        );
        assert_ne!(
            layout(
                &mut cache,
                &board,
                &BTreeSet::new(),
                Pane::Rows,
                Rect::new(0, 0, 80, 20)
            )
            .unwrap(),
            first
        );
        board.mode = BoardMode::Tabs;
        assert_eq!(
            layout(&mut cache, &board, &BTreeSet::new(), Pane::Notes, area).unwrap()[1].0,
            vec!["notes"]
        );
        assert_eq!(
            layout(&mut cache, &board, &BTreeSet::new(), Pane::Rows, area).unwrap()[1].0,
            vec!["rows"]
        );
    }
    #[test]
    fn unfolded_literal_rectangles_match_before_cutover() {
        let tree = split(
            r#"{ direction = "left-right", sizes = [30, { grow = 1 }], panes = ["rows", { direction = "top-bottom", sizes = [30, { grow = 2 }, { grow = 1 }], panes = ["detail", "notes", "replies"] }] }"#,
        );
        let node = compile(&tree, &BTreeSet::new()).unwrap();
        let area = Rect::new(2, 3, 100, 40);
        assert_eq!(
            rectangles(&node, area).unwrap(),
            [
                (Pane::Rows, Rect::new(2, 3, 30, 40)),
                (Pane::Detail, Rect::new(32, 3, 70, 12)),
                (Pane::Notes, Rect::new(32, 15, 70, 19)),
                (Pane::Replies, Rect::new(32, 34, 70, 9)),
            ]
        );
    }
    #[test]
    fn folded_literal_rectangles_match_before_cutover() {
        let tree = split(
            r#"{ direction = "left-right", panes = ["rows", { direction = "top-bottom", panes = ["detail", "notes", "replies"] }] }"#,
        );
        let folded = BTreeSet::from([Pane::Detail, Pane::Notes, Pane::Replies]);
        let node = compile(&tree, &folded).unwrap();
        assert_eq!(
            rectangles(&node, Rect::new(0, 0, 100, 20)).unwrap(),
            [
                (Pane::Rows, Rect::new(0, 0, 90, 20)),
                (Pane::Detail, Rect::new(91, 0, 9, 1)),
                (Pane::Notes, Rect::new(91, 1, 9, 1)),
                (Pane::Replies, Rect::new(91, 2, 9, 1)),
            ]
        );
        for collapsed in [
            folded,
            BTreeSet::from([Pane::Rows, Pane::Detail, Pane::Notes, Pane::Replies]),
        ] {
            let node = compile(&tree, &collapsed).unwrap();
            for w in 0..20 {
                for h in 0..6 {
                    let area = Rect::new(2, 3, w, h);
                    let solved = rectangles(&node, area).unwrap();
                    for (index, (_, rect)) in solved.iter().enumerate() {
                        assert!(rect.right() <= area.right() && rect.bottom() <= area.bottom());
                        for (_, previous) in &solved[..index] {
                            assert!(rect.intersection(*previous).is_empty());
                        }
                    }
                }
            }
        }

        let mixed = split(
            r#"{ direction = "top-bottom", sizes = [30, { grow = 100 }, { grow = 50 }], panes = ["detail", "rows", "notes"] }"#,
        );
        for (pane, height, expected) in [
            (Pane::Detail, 31, vec![1, 20, 10]),
            (Pane::Notes, 32, vec![12, 19, 1]),
        ] {
            let node = compile(&mixed, &BTreeSet::from([pane])).unwrap();
            assert_eq!(
                rectangles(&node, Rect::new(0, 0, 80, height))
                    .unwrap()
                    .iter()
                    .map(|(_, rect)| rect.height)
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
    #[test]
    fn nested_fraction_cycle_has_literal_rectangles() {
        let config = crate::config::Config::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/markup-parity.toml"),
        )
        .unwrap();
        let tree = config.board("team").unwrap().split;
        let node = compile(&tree, &BTreeSet::new()).unwrap();
        // Squad ruling req_8fd1910e; literals cover tiny and fractional-parent cycle cases.
        for (height, top, detail, replies, notes) in [
            (1, 1, 0, 1, 0),
            (8, 5, 2, 3, 3),
            (21, 13, 6, 7, 8),
            (28, 17, 8, 9, 11),
        ] {
            assert_eq!(
                rectangles(&node, Rect::new(2, 3, 120, height)).unwrap(),
                [
                    (Pane::Rows, Rect::new(2, 3, 74, top)),
                    (Pane::Detail, Rect::new(76, 3, 46, detail)),
                    (Pane::Replies, Rect::new(76, 3 + detail, 46, replies)),
                    (Pane::Notes, Rect::new(2, 3 + top, 120, notes)),
                ]
            );
        }
    }
    #[test]
    fn preset_rectangle_matrix_matches_before_cutover() {
        let config = crate::config::Config::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/markup-parity.toml"),
        )
        .unwrap();
        use sha2::{Digest, Sha256};
        let mut digest = Sha256::new();

        for preset in ["crew", "pr-queue", "minimal", "team"] {
            let board = config.board(preset).unwrap();
            for mask in 0..(1 << board.panes.len()) {
                let collapsed: BTreeSet<_> = board
                    .panes
                    .iter()
                    .enumerate()
                    .filter_map(|(i, p)| (mask & (1 << i) != 0).then_some(*p))
                    .collect();
                let node = compile(&board.split, &collapsed).unwrap();
                // Include the full fractional-parent cycle before retiring the oracle.
                for (w, h) in [0, 3, 31, 80, 99, 100, 120]
                    .into_iter()
                    .flat_map(|w| (0..=32).map(move |h| (w, h)))
                {
                    let area = Rect::new(2, 3, w, h);
                    let actual = rectangles(&node, area).unwrap();
                    // Squad ruling req_8fd1910e: half of the raw 60% parent.
                    if preset == "team" && collapsed.iter().all(|p| *p == Pane::Rows) {
                        let parent = (f32::from(h) * 0.6).round() as u16;
                        let detail = (f32::from(h) * 0.6 * 0.5).round() as u16;
                        assert_eq!(
                            (actual[1].1.height, actual[2].1.y, actual[2].1.height),
                            (detail, area.y + detail, parent - detail)
                        );
                    }
                    digest.update(format!("{preset}:{mask}:{w}:{h}:{actual:?}\n"));
                }
            }
        }
        let digest: String = digest
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        // Old solver at main40ad78c8, plus Squad's approved nested-rounding rule.
        // Captured only after all 6006 configurations matched before cutover.
        assert_eq!(
            digest,
            "dc812c42ad2b4d960861be2bae07c08d6c03c5bd0cca5f831be1c644a0b531aa"
        );
    }
}
