use super::*;
use crate::binding::{Schemas, Scopes};
use ratatui::{
    crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers},
    style::{Color, Modifier},
};
use serde_json::json;
use std::collections::BTreeMap;
use tmt_cli_style::{Depth, Theme, theme::screen};

struct NoSources;
impl Sources for NoSources {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("no source handles".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("no source handles".into())
    }
}
const TABLE: &str = r#"<tmt-view version="1"><tmt-table id="records" bind="$.rows" as="item" empty="(no records)"><tmt-row class="grid grid-cols-[2_1fr_6] gap-1"><tmt-cell id="mark" bind="item.mark" token-bind="item.role"/><tmt-cell id="text" bind="item.text" wrap="true" token="text"/><tmt-cell id="path" bind="item.path" class="truncate-middle" token="link"/></tmt-row></tmt-table></tmt-view>"#;
fn schema() -> Schema {
    Schema::Object(BTreeMap::from([
        (
            "rows".into(),
            Schema::Collection(Box::new(Schema::Object(BTreeMap::from([
                ("id".into(), Schema::StableId),
                ("disabled".into(), Schema::Boolean),
                ("mark".into(), Schema::Scalar),
                ("role".into(), Schema::Scalar),
                ("text".into(), Schema::Scalar),
                ("path".into(), Schema::Scalar),
            ])))),
        ),
        ("query".into(), Schema::Scalar),
        ("footer".into(), Schema::Scalar),
    ]))
}
fn data() -> Value {
    json!({"rows":[
        {"id":"alpha","disabled":false,"mark":"✗","role":"blocked","text":"a long wrapped description with more text","path":"long/path/end"},
        {"id":"beta","disabled":false,"mark":"!","role":"waiting","text":"second wrapped description has many words","path":"other/end"},
        {"id":"off","disabled":true,"mark":"*","role":"muted","text":"disabled row","path":"disabled"},
        {"id":"gamma","disabled":false,"mark":"✓","role":"working","text":"last row","path":"final"}
    ],"query":"find▏","footer":"↑↓ choose · Enter use · Esc close"})
}
fn surface(data: &Value) -> ListSurface {
    compile(
        "table.xml",
        &crate::parse("table.xml", TABLE).unwrap(),
        &schema(),
        &NoSources,
    )
    .unwrap()
    .materialize("table.xml", data, &NoSources)
    .unwrap()
}
fn draw(
    surface: &ListSurface,
    state: &mut ListState,
    width: u16,
    height: u16,
    depth: Depth,
) -> (Buffer, ListFrame) {
    let area = Rect::new(2, 3, width, height);
    let mut buffer = Buffer::filled(
        Rect::new(0, 0, width + 4, height + 6),
        ratatui::buffer::Cell::new("X"),
    );
    let theme = Theme::default();
    let frame = render(
        surface,
        state,
        area,
        &mut buffer,
        RenderStyle {
            theme: &theme,
            depth,
        },
        |role| {
            if depth == Depth::None {
                let attention = matches!(role, Role::Blocked | Role::Waiting | Role::Working);
                Style::new().add_modifier(
                    Modifier::REVERSED
                        | if attention {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        },
                )
            } else {
                screen::style(&theme, role, depth).bg(Color::Blue)
            }
        },
    )
    .unwrap();
    (buffer, frame)
}
fn line(buffer: &Buffer, area: Rect, y: u16) -> String {
    (area.x..area.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}
#[test]
fn table_grid_wraps_under_fixed_gutter_and_selects_every_line_edge_to_edge() {
    let mut state = ListState::default();
    let (buffer, frame) = draw(&surface(&data()), &mut state, 28, 8, Depth::TrueColor);
    let alpha = &frame.geometry[0];
    assert!(alpha.lines.len() >= 3);
    for y in alpha.visible.y..alpha.visible.bottom() {
        for x in frame.viewport.x..frame.viewport.right() {
            assert_eq!(buffer[(x, y)].bg, Color::Blue);
        }
    }
    assert_eq!(buffer[(frame.viewport.x, alpha.visible.y)].symbol(), "✗");
    assert_eq!(
        buffer[(frame.viewport.x, alpha.visible.y + 1)].symbol(),
        " "
    );
    assert_eq!(
        buffer[(frame.viewport.x + 3, alpha.visible.y + 1)].symbol(),
        "d"
    );
    assert!(line(&buffer, frame.viewport, alpha.visible.y).contains('…'));
    assert_eq!(buffer[(1, 3)].symbol(), "X");
}
#[test]
fn no_color_selection_has_common_reverse_foreground_and_bold_attention() {
    let mut state = ListState::default();
    let (buffer, frame) = draw(&surface(&data()), &mut state, 28, 8, Depth::None);
    let row = &frame.geometry[0];
    for y in row.visible.y..row.visible.bottom() {
        for x in frame.viewport.x..frame.viewport.right() {
            let cell = &buffer[(x, y)];
            assert!(cell.modifier.contains(Modifier::REVERSED));
            assert_eq!(cell.fg, Color::Reset);
        }
    }
    assert!(
        buffer[(frame.viewport.x, row.visible.y)]
            .modifier
            .contains(Modifier::BOLD)
    );
    assert!(
        !buffer[(frame.viewport.x + 3, row.visible.y)]
            .modifier
            .contains(Modifier::BOLD)
    );
}
#[test]
fn selection_reveal_refresh_resize_and_disabled_hits_follow_current_geometry() {
    let mut state = ListState::default();
    let surface = surface(&data());
    let (_, first) = draw(&surface, &mut state, 28, 4, Depth::TrueColor);
    state.input(
        &Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
        &first,
    );
    let (_, last) = draw(&surface, &mut state, 28, 4, Depth::TrueColor);
    assert_eq!(state.selected(), Some("gamma"));
    assert!(state.scroll.offset() > 0);
    assert!(!last.geometry.last().unwrap().visible.is_empty());
    for row in &last.geometry {
        assert_eq!(row.visible.intersection(last.viewport), row.visible);
    }
    let (_, wide) = draw(&surface, &mut state, 80, 20, Depth::TrueColor);
    assert_eq!(state.scroll.offset(), 0);
    assert_eq!(wide.geometry[0].lines.len(), 1);
    let empty = self::surface(&json!({"rows":[]}));
    let (buffer, empty_frame) = draw(&empty, &mut state, 28, 4, Depth::None);
    assert_eq!(state.selected(), None);
    assert_eq!(state.scroll.offset(), 0);
    assert!(empty_frame.geometry.is_empty());
    assert!(line(&buffer, empty_frame.viewport, 3).starts_with("(no records)"));
}
#[test]
fn row_admission_is_eager_and_boolean_metadata_is_strict() {
    for bad in [
        TABLE.replace("item.text", "item.unknown"),
        TABLE.replace("as=\"item\"", "as=\"42\""),
        TABLE.replace("grid-cols-[2_1fr_6]", ""),
        TABLE.replace("id=\"mark\"", "id=\"component-disabled\""),
        TABLE.replace("bind=\"$.rows\"", "bind=\"$.query\""),
    ] {
        let parsed = crate::parse("bad.xml", &bad).unwrap();
        assert!(
            compile("bad.xml", &parsed, &schema(), &NoSources).is_err(),
            "{bad}"
        );
    }
    let mut data = data();
    data["rows"][0]["disabled"] = json!("false");
    let template = compile(
        "table.xml",
        &crate::parse("table.xml", TABLE).unwrap(),
        &schema(),
        &NoSources,
    )
    .unwrap();
    assert!(
        template
            .materialize("table.xml", &data, &NoSources)
            .is_err()
    );
    let bad_schema = Schema::Object(BTreeMap::from([(
        "rows".into(),
        Schema::Collection(Box::new(Schema::Object(BTreeMap::from([
            ("id".into(), Schema::StableId),
            ("disabled".into(), Schema::Scalar),
        ])))),
    )]));
    assert!(
        compile(
            "table.xml",
            &crate::parse("table.xml", TABLE).unwrap(),
            &bad_schema,
            &NoSources
        )
        .is_err()
    );
    let mut duplicate = self::data();
    duplicate["rows"][1]["id"] = json!("alpha");
    assert!(
        template
            .materialize("table.xml", &duplicate, &NoSources)
            .is_err()
    );
}
#[test]
fn picker_has_fixed_query_footer_clipped_list_and_consumer_chosen_status_role() {
    let list = TABLE
        .split("<tmt-view version=\"1\">")
        .nth(1)
        .unwrap()
        .strip_suffix("</tmt-view>")
        .unwrap();
    let markup = format!(
        r#"<tmt-view version="1"><tmt-picker id="chooser" title="Choose" placement="center"><tmt-text slot="query" bind="$.query" token="text" class="truncate"/>{list}<tmt-text slot="footer" bind="$.footer" token="muted"/><tmt-text slot="status" token="blocked">✗ unavailable</tmt-text></tmt-picker></tmt-view>"#
    );
    let template = super::super::surface::compile(
        "picker.xml",
        &crate::parse("picker.xml", &markup).unwrap(),
        &schema(),
        &NoSources,
    )
    .unwrap();
    let surface = template
        .materialize("picker.xml", &data(), &NoSources)
        .unwrap();
    let mut state = ListState::default();
    let mut buffer = Buffer::empty(Rect::new(0, 0, 120, 20));
    let theme = Theme::default();
    let frame = super::super::surface::render_list(
        &surface,
        &mut state,
        buffer.area,
        &mut buffer,
        RenderStyle {
            theme: &theme,
            depth: Depth::TrueColor,
        },
        |_| Style::new().bg(Color::Blue),
    )
    .unwrap();
    assert!(frame.areas.outer.width <= 108 && frame.areas.outer.height <= 16);
    assert!(line(&buffer, frame.query, frame.query.y).starts_with("find▏"));
    assert!(line(&buffer, frame.areas.footer, frame.areas.footer.y).starts_with("↑↓ choose"));
    assert!(line(&buffer, frame.areas.status, frame.areas.status.y).starts_with("✗ unavailable"));
    assert_eq!(
        buffer[(frame.areas.status.x, frame.areas.status.y)].fg,
        screen::style(&theme, Role::Blocked, Depth::TrueColor)
            .fg
            .unwrap()
    );
    assert_eq!(frame.query.bottom(), frame.areas.content.y);
    assert!(
        frame
            .hits
            .iter()
            .all(|hit| hit.rect.intersection(frame.areas.content) == hit.rect)
    );
    let list = frame.list.unwrap();
    assert_eq!(list.viewport, frame.areas.content);
    assert!(
        list.geometry
            .iter()
            .all(|row| row.visible.intersection(list.viewport) == row.visible)
    );
}
#[test]
fn tiny_picker_and_ordinary_pane_rectangles_remain_bounded() {
    let surface = surface(&data());
    for width in 0..6 {
        for height in 0..6 {
            let mut state = ListState::default();
            let (_, frame) = draw(&surface, &mut state, width, height, Depth::None);
            for row in &frame.geometry {
                assert_eq!(row.visible.intersection(frame.viewport), row.visible);
            }
        }
    }
}

#[test]
fn list_template_uses_same_scroll_and_opaque_modal_selection_seam() {
    let row = r#"<tmt-list id="items" bind="$.rows"><tmt-row><tmt-cell class="w-2 shrink-0" bind="row.mark" token-bind="row.role"/><tmt-cell class="grow min-w-0" bind="row.text" wrap="true"/></tmt-row></tmt-list>"#;
    let markup = format!(
        r#"<tmt-view version="1"><tmt-modal id="jobs" title="Jobs" placement="body"><tmt-scroll id="body">{row}</tmt-scroll><tmt-text slot="footer">Esc close</tmt-text></tmt-modal></tmt-view>"#
    );
    let parsed = crate::parse("list.xml", &markup).unwrap();
    let template =
        super::super::surface::compile("list.xml", &parsed, &schema(), &NoSources).unwrap();
    let surface = template
        .materialize("list.xml", &data(), &NoSources)
        .unwrap();
    let mut state = ListState::default();
    let mut buffer = Buffer::filled(Rect::new(0, 0, 80, 12), ratatui::buffer::Cell::new("X"));
    let theme = Theme::default();
    let frame = super::super::surface::render_list(
        &surface,
        &mut state,
        buffer.area,
        &mut buffer,
        RenderStyle {
            theme: &theme,
            depth: Depth::None,
        },
        |_| Style::new().add_modifier(Modifier::REVERSED),
    )
    .unwrap();
    assert_eq!(state.selected(), Some("alpha"));
    assert_eq!(frame.areas.outer, buffer.area);
    let list = frame.list.unwrap();
    assert_eq!(list.rows.len(), 4);
    assert_eq!(list.geometry.len(), 4);
    assert!(line(&buffer, frame.areas.footer, frame.areas.footer.y).starts_with("Esc close"));
    assert!(!line(&buffer, frame.areas.content, frame.areas.content.y).contains('X'));
}

#[test]
fn oversized_selected_row_can_scroll_inside_it_without_repaint_snapping_back() {
    use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
    let mut model = data();
    model["rows"] = json!([{"id":"large","disabled":false,"mark":"✗","role":"blocked","text":"wrapped content ".repeat(30),"path":"end"}]);
    let surface = surface(&model);
    let mut state = ListState::default();
    let (_, first) = draw(&surface, &mut state, 28, 6, Depth::None);
    let mouse = Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: first.viewport.x,
        row: first.viewport.y,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(state.input(&mouse, &first), None);
    assert_eq!(state.scroll.offset(), 3);
    let (_, next) = draw(&surface, &mut state, 28, 6, Depth::None);
    assert_eq!(state.scroll.offset(), 3);
    assert_eq!(state.selected(), Some("large"));
    assert!(next.geometry[0].lines.len() > usize::from(next.viewport.height));
    assert_eq!(next.geometry[0].visible, next.viewport);
}
