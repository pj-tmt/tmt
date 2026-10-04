use super::*;
use crate::{
    binding::{self, Schema, Schemas, Scopes, Sources},
    parse,
};
use tmt_cli_style::grid::{self, Align, Overflow, Truncate};
struct Literals;
impl Sources for Literals {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("no application sources".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("no application sources".into())
    }
}
fn scene(attrs: &str, body: &str) -> Node {
    let element = parse(
        "geometry.xml",
        &format!("<tmt-view version='1' {attrs}>{body}</tmt-view>"),
    )
    .unwrap();
    binding::compile(
        "geometry.xml",
        &element,
        &Schema::Object(Default::default()),
        &Literals,
    )
    .unwrap()
    .materialize("geometry.xml", &serde_json::json!({}), &Literals)
    .unwrap()
}
fn measure(text: &str, flow: TextFlow, space: Space) -> [u16; 2] {
    // Literal Unicode calibration; geometry tests do not implement a text renderer.
    let natural = match text {
        "文件" => 4,
        "e\u{301}" => 1,
        "👩‍💻" => 2,
        _ => text.len() as u16,
    };
    let width = match space {
        Space::Cells(n) => n,
        Space::MinContent => 1,
        Space::MaxContent => natural,
    };
    let height = match flow {
        TextFlow::Wrap | TextFlow::Clamp(_) => grid::fit_lines(
            text,
            usize::from(width),
            Align::Left,
            Truncate::End,
            Overflow::Wrap {
                max_lines: if let TextFlow::Clamp(n) = flow {
                    n.min(255) as u8
                } else {
                    255
                },
            },
        )
        .len() as u16,
        _ => 1,
    };
    [natural.min(width), height]
}
fn boxes(root: &Node, width: u16) -> Vec<(String, Rect, Rect, u16, bool)> {
    layout(root, [width, 20], measure)
        .unwrap()
        .into_iter()
        .filter_map(|cell| {
            Some((
                cell.node.id.as_ref()?.last()?.clone(),
                cell.rect,
                cell.clip,
                cell.text_width,
                cell.cut,
            ))
        })
        .collect()
}
fn rect(x: i32, y: i32, width: u32, height: u32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}
#[test]
fn nested_flex_and_integer_percentages_keep_css_gaps() {
    let root = scene(
        "class='flex-row px-1 gap-2'",
        "<tmt-col id='a' class='w-[50%] shrink-0 py-1'><tmt-text id='text' class='h-[50%]'>a</tmt-text></tmt-col><tmt-cell id='b' class='grow'/>",
    );
    let cells = boxes(&root, 22);
    assert_eq!(cells[0].1, rect(1, 0, 10, 20));
    assert_eq!(cells[1].1, rect(1, 1, 10, 9));
    assert_eq!(cells[2].1, rect(13, 0, 8, 20));
    assert_eq!(cells, boxes(&root, 22));
}
#[test]
fn grid_tracks_fr_minmax_span_and_cumulative_rounding() {
    let root = scene(
        "class='grid grid-cols-[4_minmax(2,1fr)_2fr] gap-1'",
        "<tmt-cell id='a'/><tmt-cell id='b'/><tmt-cell id='c'/><tmt-cell id='span' class='col-span-3' />",
    );
    let cells = boxes(&root, 15);
    assert_eq!(cells[0].1.x, 0);
    assert_eq!(cells[0].1.width, 4);
    assert_eq!((cells[1].1.x, cells[1].1.width), (5, 3));
    assert_eq!((cells[2].1.x, cells[2].1.width), (9, 6));
    assert_eq!((cells[3].1.x, cells[3].1.width), (0, 15));
    assert_eq!(cells[3].1.y, cells[0].1.height as i32 + 1);
    let thirds = scene(
        "class='grid grid-cols-[1fr_1fr_1fr]'",
        "<tmt-cell id='a'/><tmt-cell id='b'/><tmt-cell id='c'/>",
    );
    let a = boxes(&thirds, 10);
    assert_eq!(
        a.iter().map(|c| (c.1.x, c.1.width)).collect::<Vec<_>>(),
        [(0, 3), (3, 4), (7, 3)]
    );
}
#[test]
fn grid_overflow_keeps_earlier_columns_and_four_cell_cut_guard() {
    // Already preselected tracks: Squad priority chooses these, never this module.
    let root = scene(
        "class='grid grid-cols-[6_8] gap-1'",
        "<tmt-cell id='first'/><tmt-cell id='last' class='truncate-middle'>path/to/file</tmt-cell>",
    );
    assert!(layout(&root, [11, 20], measure).unwrap()[0].overflowing);
    assert!(!layout(&root, [20, 20], measure).unwrap()[0].overflowing);
    let four = boxes(&root, 11);
    let three = boxes(&root, 10);
    assert_eq!(
        (four[0].1, four[0].2),
        (rect(0, 0, 6, 20), rect(0, 0, 6, 20))
    );
    assert_eq!(four[1].1, rect(7, 0, 8, 20));
    assert_eq!(four[1].2, rect(7, 0, 4, 20));
    assert!(four[1].4);
    assert_eq!(three[1].2.width, 0);
    assert_eq!(three[1].2.height, 0);
    assert_eq!(root.children[1].style.text_flow, TextFlow::Middle);
    let percentages = scene(
        "class='grid grid-cols-[50%_50%] gap-2'",
        "<tmt-cell id='a'/><tmt-cell id='b'/>",
    );
    let p = boxes(&percentages, 12);
    assert_eq!(
        (p[0].1.width, p[1].1.x, p[1].1.width, p[1].2.width),
        (6, 8, 6, 4)
    );
}
#[test]
fn text_measure_and_paint_budget_resize_and_ancestor_clips() {
    let root = scene(
        "class='grid grid-cols-[1fr_1fr]'",
        "<tmt-text id='wrap' wrap='true'>alpha beta gamma</tmt-text><tmt-col id='outer' class='px-1'><tmt-text id='unicode'>文件</tmt-text><tmt-text id='combining'>e&#x301;</tmt-text><tmt-text id='emoji'>👩‍💻</tmt-text></tmt-col>",
    );
    let full = boxes(&root, 12);
    assert_eq!(full[0].3, 6);
    let cells = layout(&root, [12, 20], measure).unwrap();
    let wrapped = &cells[1];
    assert!(
        wrapped.rect.height
            >= u32::from(
                measure(
                    "alpha beta gamma",
                    TextFlow::Wrap,
                    Space::Cells(wrapped.text_width)
                )[1]
            )
    );
    for cell in &cells {
        assert_eq!(cell.clip.intersect(rect(0, 0, 12, 20)), cell.clip);
    }
    assert_ne!(full, boxes(&root, 8));
    assert_eq!(full, boxes(&root, 12));
    for width in [0, 1, 2] {
        let tiny = layout(&root, [width, 0], measure).unwrap();
        assert!(tiny.iter().all(|c| c.clip.width == 0 || c.clip.height == 0));
    }
}
#[test]
fn wrapping_clamp_and_fractional_width_use_the_final_measure_budget() {
    let root = scene(
        "class='flex-col'",
        "<tmt-text id='wrap' class='w-7 shrink-0' wrap='true'>alpha beta gamma</tmt-text><tmt-text id='clamp' class='w-7 shrink-0 line-clamp-2'>alpha beta gamma</tmt-text><tmt-text id='padded' class='w-7 px-1 shrink-0' wrap='true'>abcdefghijk</tmt-text><tmt-text id='bounded' class='w-5 max-w-20 shrink-0' wrap='true'>abcdefghijk</tmt-text>",
    );
    let cells = boxes(&root, 12);
    assert_eq!((cells[0].1.height, cells[1].1.height), (3, 2));
    assert_eq!((cells[2].1.height, cells[2].3), (3, 5));
    assert_eq!((cells[3].1.height, cells[3].3), (3, 5));
    let fractional = scene(
        "class='grid grid-cols-[1fr_1fr]'",
        "<tmt-text id='a' wrap='true'>alpha beta gamma</tmt-text><tmt-text id='b'>beta</tmt-text>",
    );
    let mut measured = std::collections::BTreeMap::new();
    let output = layout(&fractional, [11, 20], |text, flow, space| {
        if let Space::Cells(width) = space {
            measured.insert(text.to_owned(), width);
        }
        measure(text, flow, space)
    })
    .unwrap();
    for cell in output
        .iter()
        .filter(|c| c.node.text.as_deref().is_some_and(|t| !t.is_empty()))
    {
        assert_eq!(
            measured.get(cell.node.text.as_deref().unwrap()),
            Some(&cell.text_width),
            "{measured:?}, text={:?}",
            cell.node.text
        );
        assert!(cell.content.width - u32::from(cell.text_width) <= 1);
    }
    let cells = boxes(&fractional, 11);
    assert_eq!((cells[0].1.width, cells[0].3, cells[1].1.width), (6, 5, 5));
    assert!(
        cells[0].1.height
            >= u32::from(measure("alpha beta gamma", TextFlow::Wrap, Space::Cells(5))[1])
    );
}
#[test]
fn flex_basis_bounds_and_shrink_have_literal_results() {
    let root = scene(
        "class='flex-row gap-2'",
        "<tmt-cell id='fixed' class='basis-8 shrink-0'/><tmt-cell id='flex' class='basis-[50%] grow min-w-2 max-w-6'/>",
    );
    let a = boxes(&root, 16);
    assert_eq!((a[0].1.width, a[1].1.x, a[1].1.width), (8, 10, 6));
    let b = boxes(&root, 12);
    assert_eq!((b[0].1.width, b[1].1.width), (8, 2));
}
#[test]
fn manually_forged_scene_limits_fail_before_engine_allocation() {
    let mut root = scene("", "");
    root.style.col_span = 0;
    assert!(layout(&root, [10, 10], measure).is_err());
    root.style.col_span = 1;
    for _ in 0..MAX_DEPTH {
        let mut parent = scene("", "");
        parent.children.push(root);
        root = parent;
    }
    assert!(
        layout(&root, [10, 10], measure)
            .unwrap_err()
            .contains("depth/node")
    );
}

fn shown(root: &Node, width: u16) -> Vec<String> {
    layout(root, [width, 4], measure)
        .unwrap()
        .iter()
        .filter_map(|cell| cell.node.text.clone().filter(|text| !text.is_empty()))
        .collect()
}
const KEY_LINE: &str = "<tmt-switch><tmt-case min='lg'><tmt-cell>full</tmt-cell></tmt-case><tmt-case min='md'><tmt-cell>short</tmt-cell></tmt-case><tmt-default><tmt-cell>min</tmt-cell></tmt-default></tmt-switch>";

#[test]
fn switch_picks_the_branch_by_width_and_boundaries_are_inclusive_at_min() {
    let root = scene("", KEY_LINE);
    for (width, expected) in [
        (1, "min"),
        (79, "min"),
        (80, "min"),
        (99, "min"),
        (100, "short"),
        (139, "short"),
        (140, "full"),
        (400, "full"),
    ] {
        assert_eq!(shown(&root, width), [expected], "width {width}");
    }
    // The same scene re-lays out at every resize; nothing is retained between calls.
    for width in [140, 99, 100, 80, 140] {
        let expected = if width >= 140 {
            "full"
        } else if width >= 100 {
            "short"
        } else {
            "min"
        };
        assert_eq!(shown(&root, width), [expected], "resize to {width}");
    }
}

#[test]
fn switch_output_equals_the_hand_written_branch_for_every_width() {
    // Case and default are not boxes: the chosen children are laid out as if written in place.
    let signature = |root: &Node, width| {
        layout(root, [width, 6], measure)
            .unwrap()
            .iter()
            .map(|cell| {
                (
                    cell.rect,
                    cell.content,
                    cell.clip,
                    cell.node.text.clone(),
                    cell.node.id.clone(),
                    cell.cut,
                )
            })
            .collect::<Vec<_>>()
    };
    let around = |inner: &str| {
        scene(
            "class='grid grid-cols-[6_1fr_8] gap-x-1'",
            &format!("<tmt-cell id='a'>aa</tmt-cell>{inner}<tmt-cell id='z'>zz</tmt-cell>"),
        )
    };
    let switch = around(
        "<tmt-switch><tmt-case min='md'><tmt-cell id='b' class='col-span-2'>wide</tmt-cell></tmt-case><tmt-default><tmt-cell id='b'>narrow</tmt-cell></tmt-default></tmt-switch>",
    );
    let wide = around("<tmt-cell id='b' class='col-span-2'>wide</tmt-cell>");
    let narrow = around("<tmt-cell id='b'>narrow</tmt-cell>");
    for width in [60, 99, 100, 130] {
        let expected = if width >= 100 { &wide } else { &narrow };
        assert_eq!(
            signature(&switch, width),
            signature(expected, width),
            "{width}"
        );
    }
}

#[test]
fn container_width_is_the_parent_content_width_and_terminal_is_the_viewport() {
    // The parent's padding is not available width: 120 cells with px-10 is 100 for its children.
    let container = scene("class='px-10'", KEY_LINE);
    assert_eq!(shown(&container, 120), ["short"]);
    assert_eq!(shown(&container, 119), ["min"]);
    assert_eq!(shown(&container, 159), ["short"]);
    assert_eq!(shown(&container, 160), ["full"]);
    // A pane narrower than the terminal decides for its own switch; `of="terminal"` ignores it.
    let pane = |of: &str| {
        scene(
            "class='flex-row'",
            &format!(
                "<tmt-col class='w-60'>{}</tmt-col><tmt-col class='grow'/>",
                KEY_LINE.replace("<tmt-switch>", &format!("<tmt-switch of='{of}'>"))
            ),
        )
    };
    assert_eq!(shown(&pane("container"), 200), ["min"]);
    assert_eq!(shown(&pane("terminal"), 200), ["full"]);
    assert_eq!(shown(&pane("terminal"), 99), ["min"]);
}

#[test]
fn a_switch_inside_a_width_chosen_branch_resolves_on_the_next_level() {
    let nested = scene(
        "",
        "<tmt-switch><tmt-case min='md'><tmt-col class='w-60'><tmt-switch><tmt-case min='lg'><tmt-cell>inner-lg</tmt-cell></tmt-case><tmt-default><tmt-cell>inner-min</tmt-cell></tmt-default></tmt-switch></tmt-col><tmt-switch of='terminal'><tmt-case min='lg'><tmt-cell>outer-lg</tmt-cell></tmt-case><tmt-default><tmt-cell>outer-md</tmt-cell></tmt-default></tmt-switch></tmt-case><tmt-default><tmt-cell>none</tmt-cell></tmt-default></tmt-switch>",
    );
    assert_eq!(shown(&nested, 90), ["none"]);
    // The inner container is the 60-cell column, so it never reaches lg.
    assert_eq!(shown(&nested, 120), ["inner-min", "outer-md"]);
    assert_eq!(shown(&nested, 200), ["inner-min", "outer-lg"]);
}

#[test]
fn hide_below_removes_only_that_element_and_keeps_the_siblings_in_place() {
    let hidden = scene(
        "class='flex-row gap-x-1'",
        "<tmt-cell class='w-4'>a</tmt-cell><tmt-cell class='w-4' hide-below='md'>b</tmt-cell><tmt-cell class='w-4'>c</tmt-cell>",
    );
    let at = |width| {
        layout(&hidden, [width, 1], measure)
            .unwrap()
            .iter()
            .filter(|cell| cell.node.text.is_some() && cell.node.kind == Kind::Cell)
            .map(|cell| (cell.node.text.clone().unwrap(), cell.rect.x))
            .collect::<Vec<_>>()
    };
    assert_eq!(at(99), [("a".into(), 0), ("c".into(), 5)]);
    assert_eq!(
        at(100),
        [("a".into(), 0), ("b".into(), 5), ("c".into(), 10)]
    );
}

#[test]
fn the_resolving_pass_measures_no_branch_text() {
    // Text is measured once per final layout: an unresolved switch has no children
    // in the pass that measures its parent, so branches never cost a second measure.
    let count = |root: &Node, width| {
        let mut calls = 0;
        layout(root, [width, 2], |text, flow, space| {
            calls += usize::from(!text.is_empty());
            measure(text, flow, space)
        })
        .unwrap();
        calls
    };
    let switch = scene(
        "",
        "<tmt-switch><tmt-case min='md'><tmt-cell>wide</tmt-cell></tmt-case><tmt-default><tmt-cell>narrow</tmt-cell></tmt-default></tmt-switch>",
    );
    let wide = scene("", "<tmt-cell>wide</tmt-cell>");
    let narrow = scene("", "<tmt-cell>narrow</tmt-cell>");
    assert_eq!(count(&switch, 100), count(&wide, 100));
    assert_eq!(count(&switch, 99), count(&narrow, 99));
}
