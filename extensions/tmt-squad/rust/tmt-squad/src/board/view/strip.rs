//! Admitted text paint for board chrome and placeholders.
use std::collections::BTreeMap;
use tmt_tui::binding::{Schema, Schemas, Scopes, Sources};

struct Display;
impl Sources for Display {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("board strips bind display text only".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("board strips acquire no sources".into())
    }
}

/// Paint a single rich-text strip through admitted TUI text. Callers retain
/// their resolved span style; geometry and terminal escaping stay in tmt-tui.
pub(super) fn paint_line(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    line: ratatui::text::Line<'_>,
    look: crate::look::Look,
) {
    use std::sync::OnceLock;
    use tmt_tui::{binding, geometry, paint};
    static TEMPLATE: OnceLock<binding::Template<()>> = OnceLock::new();
    let template = TEMPLATE.get_or_init(|| {
        let schema = Schema::Object(BTreeMap::from([("text".into(), Schema::Scalar)]));
        binding::compile("squad.strip.xml", &tmt_tui::parse("squad.strip.xml",
            r#"<tmt-view version="1"><tmt-text bind="$.text" class="w-full h-full"/></tmt-view>"#).unwrap(), &schema, &Display).unwrap()
    });
    let mut x = area.x;
    for span in line.spans {
        let width = span
            .width()
            .min(usize::from(area.right().saturating_sub(x))) as u16;
        if width == 0 {
            continue;
        }
        // The fixed schema receives an object with a string, and the fixed
        // two-node tree has no sources, loops or conditional expansion. Finite
        // u16 geometry cannot introduce invalid structure; still skip a span
        // on any future materialization/layout failure instead of panicking.
        let Ok(mut node) = template.materialize(
            "squad.strip.xml",
            &serde_json::json!({"text": span.content}),
            &Display,
        ) else {
            continue;
        };
        node.selected = true;
        let Ok(mut cells) = geometry::layout(&node, [width, area.height], tmt_tui::text::measure)
        else {
            continue;
        };
        for cell in &mut cells {
            for rect in [&mut cell.rect, &mut cell.content, &mut cell.clip] {
                rect.x += i32::from(x);
                rect.y += i32::from(area.y);
            }
        }
        paint::paint(&cells, frame.buffer_mut(), &look.theme, look.depth, |_| {
            line.style.patch(span.style)
        });
        x += width;
    }
}
