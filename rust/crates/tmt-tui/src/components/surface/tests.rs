use super::*;
use crate::{
    binding::{Schemas, Scopes},
    components::{KeyHelp, KeyHelpEntry, KeyHelpSection, Step},
};
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
};
use serde_json::json;
use std::collections::BTreeMap;
use tmt_cli_style::theme::screen;

const MARKUP: &str = r#"<tmt-view version="1"><tmt-modal id="help" title="Help" placement="body"><tmt-text slot="footer" token="muted" bind="$.footer"/><tmt-scroll id="body"><tmt-key-help id="keys" bind="$.help"/></tmt-scroll><tmt-text slot="status" token="waiting" bind="$.status"/></tmt-modal></tmt-view>"#;
struct NoSources;
impl Sources for NoSources {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("no sources".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("no sources".into())
    }
}
fn schema() -> Schema {
    Schema::Object(BTreeMap::from([
        ("help".into(), KeyHelp::schema()),
        ("footer".into(), Schema::Scalar),
        ("status".into(), Schema::Scalar),
    ]))
}
fn model() -> KeyHelp {
    KeyHelp {
        sections: vec![
            KeyHelpSection {
                id: "navigation".into(),
                title: "Navigation".into(),
                entries: vec![KeyHelpEntry {
                    id: "up".into(),
                    keys: "j".into(),
                    description: "first description".into(),
                }],
            },
            KeyHelpSection {
                id: "bindings".into(),
                title: "Bindings".into(),
                entries: vec![KeyHelpEntry {
                    id: "wide".into(),
                    keys: "界界界".into(),
                    description: "second description".into(),
                }],
            },
        ],
    }
}
fn scene(help: &KeyHelp) -> ModalSurface {
    let parsed = crate::parse("help.xml", MARKUP).unwrap();
    compile("help.xml", &parsed, &schema(), &NoSources)
        .unwrap()
        .materialize(
            "help.xml",
            &json!({"help": help.value(), "footer": "Esc close", "status": "read-only"}),
            &NoSources,
        )
        .unwrap()
}
fn draw(
    scene: &ModalSurface,
    scroll: &mut ScrollState,
    width: u16,
    height: u16,
) -> (Buffer, FrameMap) {
    let mut buffer = Buffer::filled(
        Rect::new(0, 0, width + 4, height + 4),
        ratatui::buffer::Cell::new("X"),
    );
    let theme = Theme::default();
    let map = render(
        scene,
        scroll,
        Rect::new(2, 2, width, height),
        &mut buffer,
        RenderStyle {
            theme: &theme,
            depth: Depth::TrueColor,
        },
        |role| screen::style(&theme, role, Depth::TrueColor),
    )
    .unwrap();
    (buffer, map)
}
fn line(buffer: &Buffer, area: Rect, y: u16) -> String {
    (area.x..area.x + area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

#[test]
fn opaque_reference_box_preserves_outside_base_and_anchors_footer_and_status() {
    let mut scroll = ScrollState::default();
    let (buffer, map) = draw(&scene(&model()), &mut scroll, 120, 18);
    assert_eq!(map.areas.outer, Rect::new(2, 2, 120, 18));
    assert_eq!(buffer[(1, 3)].symbol(), "X");
    for y in 2..20 {
        assert!(!line(&buffer, map.areas.outer, y).contains('X'));
    }
    assert!(line(&buffer, map.areas.footer, map.areas.footer.y).starts_with("Esc close"));
    assert!(line(&buffer, map.areas.status, map.areas.status.y).starts_with("read-only"));
    assert!(line(&buffer, map.areas.outer, 2).contains("Help"));
    assert_eq!(buffer[(2, 3)].symbol(), "│");
    assert!(
        map.hits
            .iter()
            .all(|hit| hit.rect.intersection(map.areas.content) == hit.rect)
    );
}

#[test]
fn wrapped_fixed_slots_reserve_lines_before_scroll_and_remeasure_on_resize() {
    let markup = MARKUP
        .replace("slot=\"status\"", "slot=\"status\" wrap=\"true\"")
        .replace("slot=\"footer\"", "slot=\"footer\" wrap=\"true\"");
    let status = "Saved preferences; local values can override them.";
    let footer = "Up Down scroll; Escape closes this reference.";
    let scene = compile(
        "wrapped.xml",
        &crate::parse("wrapped.xml", &markup).unwrap(),
        &schema(),
        &NoSources,
    )
    .unwrap()
    .materialize(
        "wrapped.xml",
        &json!({"help": model().value(), "footer": footer, "status": status}),
        &NoSources,
    )
    .unwrap();
    let mut scroll = ScrollState::default();
    for width in [24, 80, 24] {
        let (buffer, map) = draw(&scene, &mut scroll, width, 18);
        assert_eq!(
            map.areas.status.height,
            text::lines(status, map.areas.status.width, crate::style::TextFlow::Wrap).len() as u16
        );
        assert_eq!(
            map.areas.footer.height,
            text::lines(footer, map.areas.footer.width, crate::style::TextFlow::Wrap).len() as u16
        );
        assert!(map.areas.content.bottom() <= map.areas.status.y);
        let fixed = |area: Rect| {
            (area.y..area.bottom())
                .map(|y| line(&buffer, area, y).trim().to_owned())
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(fixed(map.areas.status), status);
        assert_eq!(fixed(map.areas.footer), footer);
        scroll.step(Step::Bottom);
        let (scrolled, next) = draw(&scene, &mut scroll, width, 18);
        assert_eq!(next.areas.footer, map.areas.footer);
        assert_eq!(
            line(&scrolled, next.areas.footer, next.areas.footer.y),
            line(&buffer, map.areas.footer, map.areas.footer.y)
        );
    }
    for height in 0..6 {
        let (_, map) = draw(&scene, &mut scroll, 3, height);
        for area in [
            map.areas.content,
            map.areas.status,
            map.areas.footer,
            map.areas.position,
        ] {
            assert!(area.height == 0 || area.intersection(map.areas.outer) == area);
        }
    }
}

#[test]
fn authored_modal_height_bounds_docked_prompt_without_changing_reference_body() {
    let scene_for = |placement| {
        let markup = MARKUP.replace(
            "placement=\"body\"",
            &format!("placement=\"{placement}\" class=\"h-7\""),
        );
        compile(
            "height.xml",
            &crate::parse("height.xml", &markup).unwrap(),
            &schema(),
            &NoSources,
        )
        .unwrap()
        .materialize(
            "height.xml",
            &json!({"help": model().value(), "footer": "Esc close", "status": "read-only"}),
            &NoSources,
        )
        .unwrap()
    };
    for height in [0, 1, 5, 20, 40] {
        let mut scroll = ScrollState::default();
        let (_, map) = draw(&scene_for("docked"), &mut scroll, 80, height);
        assert_eq!(
            map.areas.outer.height,
            7.min((u32::from(height) * 4 / 5) as u16)
        );
        assert_eq!(map.areas.outer.bottom(), height + 2);
        let (_, reference) = draw(&scene_for("body"), &mut scroll, 80, height);
        assert_eq!(reference.areas.outer, Rect::new(2, 2, 80, height));
    }
}

#[test]
fn every_section_uses_the_global_display_cell_key_width() {
    let mut scroll = ScrollState::default();
    let (buffer, map) = draw(&scene(&model()), &mut scroll, 120, 18);
    let rows: Vec<_> = (map.areas.content.y..map.areas.content.bottom())
        .filter(|y| line(&buffer, map.areas.content, *y).contains("description"))
        .collect();
    assert_eq!(rows.len(), 2);
    let x = map.areas.content.x + 8; // six display cells, then the two-cell gap.
    assert_eq!(buffer[(x, rows[0])].symbol(), "f");
    assert_eq!(buffer[(x, rows[1])].symbol(), "s");
    assert_eq!(
        map.hit_at(x, rows[0]).unwrap().row_id.as_deref(),
        Some("up")
    );
    assert_eq!(
        map.hit_at(x, rows[1]).unwrap().row_id.as_deref(),
        Some("wide")
    );
}

#[test]
fn narrow_descriptions_stack_wrap_and_keep_footer_fixed_while_scroll_clamps() {
    let mut model = model();
    model.sections[1].entries[0].description = "a long description that wraps across several visual lines and must remain under its own column".repeat(3);
    let scene = scene(&model);
    let mut scroll = ScrollState::default();
    let (buffer, map) = draw(&scene, &mut scroll, 24, 12);
    assert_eq!(
        buffer[(map.areas.content.x, map.areas.content.y + 1)].symbol(),
        "j"
    );
    assert_eq!(
        buffer[(map.areas.content.x + 1, map.areas.content.y + 2)].symbol(),
        "f"
    );
    assert!(scroll.content() > usize::from(map.areas.content.height));
    scroll.step(Step::Bottom);
    let (buffer, map) = draw(&scene, &mut scroll, 24, 12);
    assert!(scroll.offset() > 0);
    assert!(line(&buffer, map.areas.footer, map.areas.footer.y).starts_with("Esc close"));
    let (_, wide) = draw(&scene, &mut scroll, 120, 30);
    assert_eq!(scroll.offset(), 0);
    assert!(
        wide.hits
            .iter()
            .all(|hit| hit.rect.intersection(wide.areas.content) == hit.rect)
    );
    scroll.step(Step::Bottom);
    draw(
        &super::tests::scene(&KeyHelp::default()),
        &mut scroll,
        120,
        30,
    );
    assert_eq!(scroll.offset(), 0);
}

#[test]
fn scroll_navigation_mouse_bounds_and_selected_range_survive_metric_changes() {
    let mut scroll = ScrollState::default();
    scroll.update(Rect::new(5, 7, 20, 10), 100, None);
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    for code in [KeyCode::Down, KeyCode::Char('j')] {
        assert!(scroll.input(&key(code)));
    }
    assert_eq!(scroll.offset(), 2);
    scroll.input(&key(KeyCode::PageDown));
    assert_eq!(scroll.offset(), 11);
    scroll.input(&key(KeyCode::End));
    assert_eq!(scroll.offset(), 90);
    scroll.input(&key(KeyCode::Home));
    assert_eq!(scroll.offset(), 0);
    let mouse = |x| {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: x,
            row: 7,
            modifiers: KeyModifiers::NONE,
        })
    };
    assert!(!scroll.input(&mouse(4)));
    assert_eq!(scroll.offset(), 0);
    assert!(scroll.input(&mouse(5)));
    assert_eq!(scroll.offset(), 3);
    scroll.update(Rect::new(5, 7, 20, 10), 20, Some(15..18));
    assert_eq!(scroll.offset(), 8);
    scroll.update(Rect::new(5, 7, 20, 10), 2, None);
    assert_eq!(scroll.offset(), 0);
    scroll.step(Step::Pages(isize::MAX));
    assert_eq!(scroll.offset(), 0);
}

#[test]
fn tiny_rectangles_and_zero_content_stay_bounded() {
    let scene = scene(&KeyHelp::default());
    for width in 0..7 {
        for height in 0..7 {
            let mut scroll = ScrollState::default();
            let (_, map) = draw(&scene, &mut scroll, width, height);
            for hit in map.hits {
                assert_eq!(hit.rect.intersection(map.areas.outer), hit.rect);
            }
            assert_eq!(scroll.offset(), 0);
        }
    }
}

#[test]
fn malformed_components_and_empty_collection_schemas_fail_before_render() {
    for markup in [
        MARKUP.replace("placement=\"body\"", "placement=\"unknown\""),
        MARKUP.replace("id=\"body\"", ""),
        MARKUP.replace("$.help", "$.unknown"),
        MARKUP.replace("slot=\"status\"", "slot=\"footer\""),
        MARKUP.replace("<tmt-key-help", "<tmt-scroll id=\"nested\"/><tmt-key-help"),
    ] {
        let parsed = crate::parse("bad.xml", &markup).unwrap();
        assert!(compile("bad.xml", &parsed, &schema(), &NoSources).is_err());
    }
    let parsed = crate::parse("help.xml", MARKUP).unwrap();
    assert!(binding::compile("help.xml", &parsed, &schema(), &NoSources).is_err());
    let mut incomplete = schema();
    if let Schema::Object(fields) = &mut incomplete {
        fields.insert("help".into(), Schema::Collection(Box::new(Schema::Scalar)));
    }
    assert!(compile("help.xml", &parsed, &incomplete, &NoSources).is_err());
}

#[test]
fn small_modal_is_centered_and_capped_but_narrow_modal_fills_width() {
    let modal = Modal {
        title: "Confirm".into(),
        placement: Placement::Center,
    };
    let areas = modal.areas(Rect::new(10, 5, 120, 40), [200, 200], true, true);
    assert_eq!(areas.outer, Rect::new(16, 9, 108, 32));
    assert_eq!(areas.footer.y, areas.outer.bottom() - 2);
    let narrow = modal.areas(Rect::new(10, 5, 80, 40), [10, 10], true, false);
    assert_eq!(narrow.outer.width, 80);
    let docked = Modal {
        placement: Placement::Docked,
        ..modal
    }
    .areas(Rect::new(10, 5, 80, 40), [30, 10], true, false);
    assert_eq!(docked.outer.bottom(), 45);
}

#[test]
fn footer_keeps_whole_hints_by_priority_and_escapes_controls() {
    use crate::components::{KeyHint, footer};
    let hints = [
        KeyHint {
            key: "?".into(),
            description: "more".into(),
        },
        KeyHint {
            key: "q".into(),
            description: "quit".into(),
        },
        KeyHint {
            key: "界".into(),
            description: "open".into(),
        },
    ];
    assert_eq!(footer(&hints, 6, "  "), "? more");
    assert_eq!(footer(&hints, 14, "  "), "? more  q quit");
    assert_eq!(footer(&hints, 5, "  "), "");
    let escaped = footer(
        &[KeyHint {
            key: "x".into(),
            description: "\u{1b}[31m".into(),
        }],
        80,
        " · ",
    );
    assert!(!escaped.contains('\u{1b}'));
}

#[test]
fn component_generated_depth_is_rejected_during_admission() {
    let markup = format!(
        "<tmt-view version='1'><tmt-modal id='m' title='Help'><tmt-scroll id='s'>{}<tmt-key-help id='k' bind='$.help'/>{}</tmt-scroll></tmt-modal></tmt-view>",
        "<tmt-col>".repeat(25),
        "</tmt-col>".repeat(25)
    );
    let parsed = crate::parse("deep.xml", &markup).unwrap();
    let error = compile("deep.xml", &parsed, &schema(), &NoSources)
        .err()
        .unwrap();
    assert!(error.message.contains("depth/node limits"));
    assert_eq!(error.file, "deep.xml");
}

#[test]
fn semantic_help_styles_and_escaping_use_injected_theme_at_each_depth() {
    let mut help = model();
    help.sections[0].entries[0].description = "text\u{1b}[31m".into();
    let scene = scene(&help);
    for depth in [Depth::TrueColor, Depth::Ansi16, Depth::None] {
        let theme = Theme::default();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 120, 20));
        let mut scroll = ScrollState::default();
        let map = render(
            &scene,
            &mut scroll,
            buffer.area,
            &mut buffer,
            RenderStyle {
                theme: &theme,
                depth,
            },
            |role| screen::style(&theme, role, depth),
        )
        .unwrap();
        let y = map.areas.content.y + 1;
        let expected = |role| {
            let mut cell = ratatui::buffer::Cell::default();
            cell.set_style(screen::style(&theme, role, depth));
            cell.style()
        };
        assert_eq!(
            buffer[(map.areas.content.x, y)].style(),
            expected(Role::Accent)
        );
        assert_eq!(
            buffer[(map.areas.content.x + 8, y)].style(),
            expected(Role::Text)
        );
        for cell in &buffer.content {
            assert!(!cell.symbol().contains('\u{1b}'));
        }
        assert_eq!(
            buffer[(map.areas.position.x, map.areas.position.y)].style(),
            expected(Role::Muted)
        );
    }
}

#[test]
fn key_help_heading_properties_preserve_columns_and_section_spacing() {
    let markup = MARKUP.replace(
        "bind=\"$.help\"",
        "bind=\"$.help\" heading-token=\"accent\" heading-bold=\"true\" section-gap=\"1\"",
    );
    let parsed = crate::parse("help.xml", &markup).unwrap();
    let surface = compile("help.xml", &parsed, &schema(), &NoSources)
        .unwrap()
        .materialize(
            "help.xml",
            &json!({"help": model().value(), "footer": "Esc close", "status": "read-only"}),
            &NoSources,
        )
        .unwrap();
    let mut scroll = ScrollState::default();
    let (buffer, map) = draw(&surface, &mut scroll, 120, 18);
    let x = map.areas.content.x;
    let y = map.areas.content.y;
    assert!(line(&buffer, map.areas.content, y).starts_with("Navigation"));
    assert!(line(&buffer, map.areas.content, y + 2).trim().is_empty());
    assert!(line(&buffer, map.areas.content, y + 3).starts_with("Bindings"));
    assert_eq!(
        buffer[(x, y)].fg,
        screen::style(&Theme::default(), Role::Accent, Depth::TrueColor)
            .fg
            .unwrap()
    );
    assert!(
        buffer[(x, y)]
            .modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
    assert!(
        !buffer[(x, y + 1)]
            .modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
    assert_eq!(buffer[(x + 8, y + 1)].symbol(), "f");
    assert_eq!(buffer[(x + 8, y + 4)].symbol(), "s");
    assert_eq!(scroll.content(), 5);
    let (_, small) = draw(&surface, &mut scroll, 120, 7);
    scroll.step(Step::Bottom);
    let (buffer, end) = draw(&surface, &mut scroll, 120, 7);
    assert_eq!(scroll.offset(), 5 - usize::from(small.areas.content.height));
    assert!(
        line(&buffer, end.areas.content, end.areas.content.bottom() - 1)
            .contains("second description")
    );
}

#[test]
fn invalid_key_help_heading_properties_are_rejected() {
    for property in [
        "heading-token='title'",
        "heading-bold='yes'",
        "section-gap='-1'",
        "section-gap='4097'",
    ] {
        let markup = MARKUP.replace("bind=\"$.help\"", &format!("bind=\"$.help\" {property}"));
        let parsed = crate::parse("help.xml", &markup).unwrap();
        assert!(
            compile("help.xml", &parsed, &schema(), &NoSources).is_err(),
            "{property}"
        );
    }
}
