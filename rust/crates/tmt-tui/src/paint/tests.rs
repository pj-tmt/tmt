use super::*;
use crate::{
    binding::{self, Schema, Schemas, Scopes, Sources},
    geometry, parse,
    style::TextFlow,
};
use ratatui::{
    layout::Rect as Area,
    style::{Color, Modifier},
};
use tmt_cli_style::Base;
struct Literals;
impl Sources for Literals {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("no sources".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("no sources".into())
    }
}
fn scene(attrs: &str, body: &str) -> binding::Node {
    binding::compile(
        "paint.xml",
        &parse(
            "paint.xml",
            &format!("<tmt-view version='1' {attrs}>{body}</tmt-view>"),
        )
        .unwrap(),
        &Schema::Object(Default::default()),
        &Literals,
    )
    .unwrap()
    .materialize("paint.xml", &serde_json::json!({}), &Literals)
    .unwrap()
}
fn row(buffer: &Buffer, y: u16) -> Vec<&str> {
    (buffer.area.x..buffer.area.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}
fn draw(root: &binding::Node, size: [u16; 2]) -> (Buffer, Vec<Hit<'_>>) {
    let cells = geometry::layout(root, size, text::measure).unwrap();
    let mut buffer = Buffer::empty(Area::new(0, 0, size[0], size[1]));
    let hits = paint(&cells, &mut buffer, &Theme::default(), Depth::None, |_| {
        Style::new().add_modifier(Modifier::REVERSED)
    });
    (buffer, hits)
}
#[test]
fn grapheme_cuts_wrap_clamp_and_controls_share_metrics() {
    for (value, width, flow, expected) in [
        ("文件更多", 5, TextFlow::Truncate, vec!["文件…"]),
        ("e\u{301}abc", 3, TextFlow::Truncate, vec!["e\u{301}a…"]),
        ("👩‍💻🇹🇼xyz", 5, TextFlow::Middle, vec!["👩‍💻…yz"]),
        ("abcdefg", 6, TextFlow::Middle, vec!["abc…fg"]),
        ("文件", 3, TextFlow::Clip, vec!["文 "]),
        (
            "alpha beta gamma",
            7,
            TextFlow::Wrap,
            vec!["alpha  ", "beta   ", "gamma  "],
        ),
        (
            "alpha beta gamma",
            7,
            TextFlow::Clamp(2),
            vec!["alpha  ", "beta g…"],
        ),
        ("\x1b[31m", 10, TextFlow::Clip, vec!["\\u{1b}[31m"]),
    ] {
        assert_eq!(text::lines(value, width, flow), expected);
        assert_eq!(
            text::measure(value, flow, geometry::Space::Cells(width))[1],
            expected.len() as u16
        );
    }
    assert_eq!(
        text::measure("👩‍💻🇹🇼e\u{301}", TextFlow::Clip, geometry::Space::MaxContent),
        [5, 1]
    );
    assert_eq!(
        text::measure("ab cdef", TextFlow::Wrap, geometry::Space::MinContent),
        [4, 2]
    );
    assert!(text::lines("a", 0, TextFlow::Wrap).is_empty());
}
#[test]
fn recorded_width_preserves_fractional_blanks_and_wrapped_height() {
    let root = scene(
        "class='grid grid-cols-[1fr_1fr]'",
        "<tmt-text id='a' wrap='true'>abcd</tmt-text><tmt-text id='b'>z</tmt-text>",
    );
    let cells = geometry::layout(&root, [5, 4], text::measure).unwrap();
    assert_eq!(
        (
            cells[1].content.width,
            cells[1].text_width,
            cells[1].rect.height
        ),
        (3, 2, 4)
    );
    let (buffer, hits) = draw(&root, [5, 4]);
    assert_eq!(row(&buffer, 0), ["a", "b", " ", "z", " "]);
    assert_eq!(row(&buffer, 1), ["c", "d", " ", " ", " "]);
    assert_eq!(hit_at(&hits, 2, 1).unwrap().id.unwrap(), ["a"]);
    let padded = scene(
        "",
        "<tmt-text class='w-7 px-1 shrink-0' wrap='true'>abcdefgh</tmt-text>",
    );
    let cells = geometry::layout(&padded, [8, 4], text::measure).unwrap();
    assert_eq!((cells[1].text_width, cells[1].rect.height), (5, 2));
    assert_eq!(
        row(&draw(&padded, [8, 4]).0, 1),
        [" ", "f", "g", "h", " ", " ", " ", " "]
    );
}
#[test]
fn cut_lines_keep_logical_height_without_rewrapping() {
    let root = scene(
        "class='grid grid-cols-[4_8]'",
        "<tmt-cell>head</tmt-cell><tmt-row id='cut' row-id='member' class='flex-col'><tmt-text wrap='true'>abcdefgh ijklmnop</tmt-text></tmt-row>",
    );
    let (buffer, hits) = draw(&root, [8, 4]);
    assert_eq!(row(&buffer, 0), ["h", "e", "a", "d", "a", "b", "c", "…"]);
    assert_eq!(row(&buffer, 1), [" ", " ", " ", " ", "i", "j", "k", "…"]);
    assert_eq!(hit_at(&hits, 7, 1).unwrap().row_id, Some("member"));
    assert!(
        draw(&root, [7, 4])
            .1
            .iter()
            .all(|h| h.row_id != Some("member"))
    );
    let full = draw(&root, [12, 4]);
    assert_eq!(
        row(&full.0, 0)[4..],
        ["a", "b", "c", "d", "e", "f", "g", "h"]
    );
}
#[test]
fn nested_hits_resize_and_buffer_offsets_keep_identity_and_wide_edges_blank() {
    let root = scene(
        "id='outer'",
        "<tmt-row id='inner' row-id='member' class='w-5 px-1 flex-col'><tmt-text id='leaf'>文件</tmt-text></tmt-row>",
    );
    for width in [120, 80, 120, 1, 0] {
        let (_, hits) = draw(&root, [width, 3]);
        assert!(hits.iter().all(|h| h.rect.width > 0 && h.rect.height > 0));
        if width > 5 {
            assert_eq!(
                hit_at(&hits, 1, 0).unwrap().id.unwrap(),
                ["outer", "inner", "leaf"]
            );
            assert_eq!(hit_at(&hits, 0, 0).unwrap().id.unwrap(), ["outer", "inner"]);
            assert_eq!(hit_at(&hits, 6, 0).unwrap().id.unwrap(), ["outer"]);
        }
    }
    let root = scene("", "<tmt-text id='wide'>文件</tmt-text>");
    let cells = geometry::layout(&root, [4, 1], text::measure).unwrap();
    let mut buffer = Buffer::empty(Area::new(1, 0, 2, 1));
    let hits = paint(&cells, &mut buffer, &Theme::default(), Depth::None, |_| {
        Style::new()
    });
    assert_eq!(row(&buffer, 0), [" ", " "]);
    assert_eq!(
        hits[0].rect,
        Rect {
            x: 1,
            y: 0,
            width: 2,
            height: 1
        }
    );
    assert!(hit_at(&hits, 0, 0).is_none());
}
#[test]
fn themes_inheritance_overrides_depth_and_caller_selection_are_exact() {
    let root = scene(
        "token='dim'",
        "<tmt-text>文件</tmt-text><tmt-text token='waiting'>x</tmt-text>",
    );
    for (base, depth, fg, modifier) in [
        (
            Base::Tmt,
            Depth::TrueColor,
            Color::Rgb(122, 131, 174),
            Modifier::empty(),
        ),
        (
            Base::TmtLight,
            Depth::TrueColor,
            Color::Rgb(104, 112, 154),
            Modifier::empty(),
        ),
        (Base::Terminal, Depth::Ansi16, Color::Reset, Modifier::DIM),
        (Base::Mono, Depth::TrueColor, Color::Reset, Modifier::DIM),
        (Base::Tmt, Depth::None, Color::Reset, Modifier::empty()),
    ] {
        let cells = geometry::layout(&root, [4, 3], text::measure).unwrap();
        let mut buffer = Buffer::empty(Area::new(0, 0, 4, 3));
        paint(&cells, &mut buffer, &Theme::new(base), depth, |_| {
            panic!("unselected")
        });
        assert_eq!(buffer[(0, 0)].symbol(), "文");
        for x in 0..4 {
            assert_eq!((buffer[(x, 0)].fg, buffer[(x, 0)].modifier), (fg, modifier));
        }
    }
    let root = scene(
        "selected='true'",
        "<tmt-text token='dim' class='w-3'>文件</tmt-text>",
    );
    let cells = geometry::layout(&root, [4, 2], text::measure).unwrap();
    let mut buffer = Buffer::empty(Area::new(0, 0, 4, 2));
    let selected = Style::new().add_modifier(Modifier::REVERSED);
    let mut roles = Vec::new();
    paint(
        &cells,
        &mut buffer,
        &Theme::default(),
        Depth::None,
        |role| {
            roles.push(role);
            selected
        },
    );
    assert_eq!(roles, [Role::Text, Role::Dim]);
    assert!(
        buffer
            .content
            .iter()
            .all(|c| c.modifier == Modifier::REVERSED && c.fg == Color::Reset)
    );
    let root = scene("token='waiting'", "<tmt-text>x</tmt-text>");
    let cells = geometry::layout(&root, [2, 1], text::measure).unwrap();
    paint(
        &cells,
        &mut buffer,
        &Theme::parse("theme", [("waiting", "green")]).unwrap(),
        Depth::Ansi16,
        |_| Style::new(),
    );
    assert_eq!(buffer[(0, 0)].fg, Color::Green);
}

#[test]
fn later_sibling_wins_and_reordering_keeps_semantic_identity() {
    let mut root = scene(
        "class='flex-row'",
        "<tmt-text id='a'>aa</tmt-text><tmt-text id='b'>bb</tmt-text>",
    );
    for expected in ["b", "a"] {
        let mut cells = geometry::layout(&root, [4, 1], text::measure).unwrap();
        cells[2].rect = cells[1].rect;
        cells[2].content = cells[1].content;
        cells[2].clip = cells[1].clip;
        let mut buffer = Buffer::empty(Area::new(0, 0, 4, 1));
        let hits = paint(&cells, &mut buffer, &Theme::default(), Depth::None, |_| {
            Style::new()
        });
        assert_eq!(hit_at(&hits, 0, 0).unwrap().id.unwrap(), [expected]);
        assert_eq!(buffer[(0, 0)].symbol(), expected);
        root.children.reverse();
    }
}

#[test]
fn aligned_board_text_keeps_graphemes_and_intrinsic_width_upper_bound() {
    use tmt_cli_style::grid::Align;
    for (value, width, flow, align, expected) in [
        (
            "e\u{301}",
            4,
            TextFlow::Truncate,
            Align::Right,
            "   e\u{301}",
        ),
        ("👩‍💻", 5, TextFlow::Middle, Align::Center, " 👩‍💻  "),
        ("👩‍💻xyz", 4, TextFlow::Truncate, Align::Right, "👩‍💻x…"),
        ("文", 1, TextFlow::Truncate, Align::Left, "…"),
        ("x ", 4, TextFlow::Truncate, Align::Right, "  x "),
    ] {
        assert_eq!(text::fit_line(value, width, flow, align), expected);
    }
    assert_eq!(
        text::fit_lines("one two three", 6, TextFlow::Clamp(2), Align::Right),
        ["   one", "two t…"]
    );
    assert_eq!(
        text::measure("a a a", TextFlow::Wrap, geometry::Space::Cells(4)),
        [4, 2]
    );
    assert_eq!(text::lines("a a a", 4, TextFlow::Wrap), ["a a ", "a   "]);
    assert_eq!(
        text::fit_line("abcdef", 0, TextFlow::Truncate, Align::Center),
        ""
    );
}

#[test]
fn caller_decoration_aligns_at_recorded_width_and_retains_scoped_hits() {
    let root = scene(
        "class='grid grid-cols-[1fr_1fr]'",
        "<tmt-text id='left' token='waiting'>x</tmt-text><tmt-text id='right' token='review'>y</tmt-text>",
    );
    let cells = geometry::layout(&root, [9, 1], text::measure).unwrap();
    let mut buffer = Buffer::empty(Area::new(0, 0, 9, 1));
    let hits = paint_with(&cells, &mut buffer, |index, role, selected| {
        assert!(!selected);
        if index == 1 {
            assert_eq!(role, Role::Waiting);
        }
        if index == 2 {
            assert_eq!(role, Role::Review);
        }
        (Style::new().fg(Color::Green), Align::Right)
    });
    assert_eq!(
        row(&buffer, 0),
        [" ", " ", " ", "x", " ", " ", " ", " ", "y"]
    );
    assert_eq!(buffer[(4, 0)].fg, Color::Green);
    assert_eq!(hit_at(&hits, 4, 0).unwrap().id.unwrap(), ["left"]);
    assert_eq!(hit_at(&hits, 8, 0).unwrap().id.unwrap(), ["right"]);
}
