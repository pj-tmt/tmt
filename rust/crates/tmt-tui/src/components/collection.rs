//! Admitted row/cell templates lowered onto the existing binding, grid and paint
//! pipeline. The same list renderer serves ordinary panes and modal bodies.
use super::{ListFrame, ListRow, ListState, RowGeometry, surface::RenderStyle};
use crate::{
    Error, Kind, MarkupElement,
    binding::{self, Node, Schema, Sources},
    geometry::{self, Cell},
    paint,
    style::{Direction, Display, Extent},
    text,
};
use ratatui::{buffer::Buffer, layout::Rect, style::Style};
use serde_json::Value;
use tmt_cli_style::Role;

const DISABLED: &str = "component-disabled";

#[derive(Clone)]
pub(crate) struct ListSpec {
    pub id: Vec<String>,
    empty: String,
}
fn fail(file: &str, element: &MarkupElement, message: impl Into<String>) -> Error {
    Error {
        file: file.into(),
        location: element.location,
        message: message.into(),
    }
}

/// Rows bind objects with an `id: StableId`, `disabled: Boolean`, and caller
/// display fields. Exactly one authored row template is eagerly admitted.
pub(crate) fn lower(
    file: &str,
    element: &mut MarkupElement,
    id: &[String],
    schema: &Schema,
) -> Result<ListSpec, Error> {
    let path = element
        .attributes
        .remove("bind")
        .ok_or_else(|| fail(file, element, "list/table requires bind"))?;
    let Schema::Collection(item) =
        binding::root_schema(schema, &path).map_err(|why| fail(file, element, why))?
    else {
        return Err(fail(file, element, "list/table bind requires a collection"));
    };
    let Schema::Object(fields) = item.as_ref() else {
        return Err(fail(file, element, "list/table rows must be objects"));
    };
    if !matches!(fields.get("id"), Some(Schema::StableId))
        || !matches!(fields.get("disabled"), Some(Schema::Boolean))
    {
        return Err(fail(
            file,
            element,
            "rows require id: StableId and disabled: Boolean",
        ));
    }
    let alias = element
        .attributes
        .remove("as")
        .unwrap_or_else(|| "row".into());
    let empty = element
        .attributes
        .remove("empty")
        .unwrap_or_else(|| "(no items)".into());
    if element.style.display == Display::Grid {
        return Err(fail(
            file,
            element,
            "grid tracks belong to the table row, not its container",
        ));
    }
    if element.children.len() != 1 || element.children[0].kind != Kind::Row {
        return Err(fail(
            file,
            element,
            "list/table requires exactly one tmt-row template",
        ));
    }
    let mut row = element.children.remove(0);
    if element.kind == Kind::Table
        && (row.style.display != Display::Grid || row.style.columns.is_empty())
    {
        return Err(fail(file, &row, "table row requires explicit grid tracks"));
    }
    if ["id", "id-bind", "row-id", "row-bind", "selected"]
        .iter()
        .any(|key| row.attributes.contains_key(*key))
    {
        return Err(fail(
            file,
            &row,
            "list/table owns row identity and selection",
        ));
    }
    fn reserved(file: &str, element: &MarkupElement) -> Result<(), Error> {
        if ["selected", "row-id", "row-bind"]
            .iter()
            .any(|key| element.attributes.contains_key(*key))
        {
            return Err(fail(
                file,
                element,
                "list/table owns row identity and selection",
            ));
        }
        if element
            .attributes
            .get("id")
            .is_some_and(|id| id == DISABLED)
        {
            return Err(fail(
                file,
                element,
                "component-disabled is a reserved row field",
            ));
        }
        if matches!(
            element.kind,
            Kind::Modal | Kind::Scroll | Kind::KeyHelp | Kind::List | Kind::Table | Kind::Picker
        ) {
            return Err(fail(
                file,
                element,
                "row templates may contain primitives only",
            ));
        }
        for child in &element.children {
            reserved(file, child)?;
        }
        Ok(())
    }
    reserved(file, &row)?;
    row.attributes
        .insert("id-bind".into(), format!("{alias}.id"));
    row.attributes
        .insert("row-bind".into(), format!("{alias}.id"));
    row.style.width = Extent::Full;
    let mut disabled = crate::parse(
        file,
        &format!(
            r#"<tmt-view version="1"><tmt-text id="{DISABLED}" bind="$.disabled"/></tmt-view>"#
        ),
    )?
    .children
    .remove(0);
    disabled.location = row.location;
    disabled
        .attributes
        .insert("bind".into(), format!("{alias}.disabled"));
    row.children.insert(0, disabled);
    let mut repeat = crate::parse(
        file,
        r#"<tmt-view version="1"><tmt-repeat each="$.rows" as="row"/></tmt-view>"#,
    )?
    .children
    .remove(0);
    repeat.location = element.location;
    repeat.attributes.insert("each".into(), path);
    repeat.attributes.insert("as".into(), alias);
    repeat.children.push(row);
    element.children.push(repeat);
    element.kind = Kind::Col;
    element.style.direction = Direction::Column;
    Ok(ListSpec {
        id: id.to_vec(),
        empty,
    })
}

fn find_mut<'a>(node: &'a mut Node, id: &[String]) -> Option<&'a mut Node> {
    if node.id.as_deref() == Some(id) {
        return Some(node);
    }
    node.children
        .iter_mut()
        .find_map(|child| find_mut(child, id))
}

pub(crate) fn extract(
    file: &str,
    node: &mut Node,
    spec: &ListSpec,
    location: crate::Location,
) -> Result<Vec<ListRow>, Error> {
    let error = |message: &str| Error {
        file: file.into(),
        location,
        message: message.into(),
    };
    let list = find_mut(node, &spec.id).ok_or_else(|| error("missing materialized list"))?;
    let mut rows = Vec::new();
    for row in &mut list.children {
        let marker = row.children.remove(0);
        let disabled = match marker.text.as_deref() {
            Some("true") => true,
            Some("false") => false,
            _ => return Err(error("row.disabled must be a boolean")),
        };
        rows.push(ListRow {
            id: row
                .row_id
                .clone()
                .ok_or_else(|| error("missing stable row ID"))?,
            disabled,
        });
        if disabled {
            fn mute(node: &mut Node) {
                if !matches!(
                    node.style.token,
                    Some(Role::Blocked | Role::Waiting | Role::Working)
                ) {
                    node.style.token = Some(Role::Muted);
                }
                for child in &mut node.children {
                    mute(child);
                }
            }
            mute(row);
        }
    }
    if rows.is_empty() {
        list.text = Some(spec.empty.clone());
        list.style.token = Some(Role::Muted);
    }
    Ok(rows)
}

pub(crate) fn select(node: &mut Node, spec: &ListSpec, selected: Option<&str>) {
    if let Some(list) = find_mut(node, &spec.id) {
        for row in &mut list.children {
            row.selected = row.row_id.as_deref() == selected;
        }
    }
}

pub(crate) fn publish(
    cells: &[Cell<'_>],
    list_id: &[String],
    scroll: &mut super::ScrollState,
    rows: &[ListRow],
    selected: Option<&str>,
    area: Rect,
    content: usize,
) -> ListFrame {
    let mut geometry = Vec::new();
    let mut clips = Vec::new();
    for cell in cells {
        if !cell
            .node
            .id
            .as_ref()
            .is_some_and(|id| id.starts_with(list_id) && id.len() == list_id.len() + 1)
        {
            continue;
        }
        let Some(id) = cell.node.row_id.as_ref() else {
            continue;
        };
        // Only the explicit row wrapper owns the visual range, not its cells.
        let start = cell.rect.y.max(0) as usize;
        clips.push(cell.rect.intersect(cell.clip));
        geometry.push(RowGeometry {
            id: id.clone(),
            lines: start..start.saturating_add(cell.rect.height as usize),
            visible: Rect::new(area.x, area.y, 0, 0),
        });
    }
    let selected = geometry
        .iter()
        .find(|row| Some(row.id.as_str()) == selected)
        .map(|row| row.lines.clone());
    scroll.update(area, content, selected);
    for (row, clip) in geometry.iter_mut().zip(clips) {
        let viewport = geometry::Rect {
            x: 0,
            y: scroll.offset().min(i32::MAX as usize) as i32,
            width: u32::from(area.width),
            height: u32::from(area.height),
        };
        let visible = clip.intersect(viewport);
        if visible.width > 0 && visible.height > 0 {
            row.visible = Rect::new(
                area.x.saturating_add(visible.x.max(0) as u16),
                area.y
                    .saturating_add((visible.y as usize - scroll.offset()) as u16),
                visible.width as u16,
                visible.height as u16,
            )
            .intersection(area);
        }
    }
    ListFrame {
        rows: rows.to_vec(),
        geometry,
        viewport: area,
        offset: scroll.offset(),
    }
}

pub(crate) fn translate(cells: &mut [Cell<'_>], area: Rect, offset: usize) {
    let clip = geometry::Rect {
        x: i32::from(area.x),
        y: i32::from(area.y),
        width: u32::from(area.width),
        height: u32::from(area.height),
    };
    for cell in cells {
        for rect in [&mut cell.rect, &mut cell.content, &mut cell.clip] {
            rect.x = rect.x.saturating_add(i32::from(area.x));
            rect.y = rect
                .y
                .saturating_add(i32::from(area.y))
                .saturating_sub(offset.min(i32::MAX as usize) as i32);
        }
        cell.clip = cell.clip.intersect(clip);
    }
}
pub(crate) fn prepare(node: &mut Node) {
    node.style.shrink = 0;
    node.style.grow = 0;
    for child in &mut node.children {
        prepare(child);
    }
}
pub(crate) fn height(cells: &[Cell<'_>]) -> Result<usize, String> {
    let height = cells
        .iter()
        .skip(1)
        .map(|cell| i64::from(cell.rect.y) + i64::from(cell.rect.height))
        .max()
        .unwrap_or(0)
        .max(0) as usize;
    if height > usize::from(u16::MAX) {
        return Err("list content exceeds 65535 visual lines".into());
    }
    Ok(height)
}

pub struct Table<S> {
    binding: binding::Template<S>,
    spec: ListSpec,
    location: crate::Location,
}
pub struct ListSurface {
    node: Node,
    rows: Vec<ListRow>,
    spec: ListSpec,
}
/// Compile one list or table as an ordinary pane, without modal chrome. Modal
/// lists and pickers use surface::compile and the identical row lowering.
pub fn compile<A: Sources>(
    file: &str,
    parsed: &MarkupElement,
    schema: &Schema,
    sources: &A,
) -> Result<Table<A::Source>, Error> {
    if parsed.kind != Kind::View
        || parsed.children.len() != 1
        || !matches!(parsed.children[0].kind, Kind::List | Kind::Table)
    {
        return Err(fail(
            file,
            parsed,
            "collection surface requires one list or table",
        ));
    }
    if parsed.attributes.contains_key("id-bind") {
        return Err(fail(
            file,
            parsed,
            "collection root requires literal identity",
        ));
    }
    let mut lowered = parsed.clone();
    let element = &mut lowered.children[0];
    if element.attributes.contains_key("id-bind") || !element.attributes.contains_key("id") {
        return Err(fail(
            file,
            element,
            "list/table requires a literal stable ID",
        ));
    }
    let mut ids = Vec::new();
    if let Some(id) = parsed.attributes.get("id") {
        ids.push(id.clone());
    }
    ids.push(element.attributes["id"].clone());
    let location = element.location;
    let spec = lower(file, element, &ids, schema)?;
    super::surface::bounds(file, &lowered, 0, &mut 0)?;
    Ok(Table {
        binding: binding::compile(file, &lowered, schema, sources)?,
        spec,
        location,
    })
}
impl<S> Table<S> {
    pub fn materialize<A: Sources<Source = S>>(
        &self,
        file: &str,
        data: &Value,
        sources: &A,
    ) -> Result<ListSurface, Error> {
        let mut node = self.binding.materialize(file, data, sources)?;
        let rows = extract(file, &mut node, &self.spec, self.location)?;
        Ok(ListSurface {
            node,
            rows,
            spec: self.spec.clone(),
        })
    }
}
pub fn render(
    surface: &ListSurface,
    state: &mut ListState,
    area: Rect,
    buffer: &mut Buffer,
    style: RenderStyle<'_>,
    selection: impl FnMut(Role) -> Style,
) -> Result<ListFrame, String> {
    state.reconcile(surface.rows.clone())?;
    let mut node = surface.node.clone();
    select(&mut node, &surface.spec, state.selected());
    prepare(&mut node);
    node.style.height = Extent::Cells(u16::MAX);
    let area = area.intersection(buffer.area);
    let mut cells = geometry::layout(&node, [area.width, u16::MAX], text::measure)?;
    let height = height(&cells)?;
    let selected = state.selected().map(str::to_owned);
    let rows = state.rows().to_vec();
    let viewport = Rect {
        height: area
            .height
            .saturating_sub(u16::from(height > usize::from(area.height))),
        ..area
    };
    let frame = publish(
        &cells,
        &surface.spec.id,
        &mut state.scroll,
        &rows,
        selected.as_deref(),
        viewport,
        height,
    );
    translate(&mut cells, viewport, state.scroll.offset());
    paint::paint(&cells, buffer, style.theme, style.depth, selection);
    let below = height.saturating_sub(
        state
            .scroll
            .offset()
            .saturating_add(usize::from(viewport.height)),
    );
    if viewport.height < area.height {
        let indicator = Rect::new(area.x, viewport.bottom(), area.width, 1);
        let role = tmt_cli_style::theme::screen::style(style.theme, Role::Dim, style.depth);
        for x in indicator.x..indicator.right() {
            buffer[(x, indicator.y)].reset();
            buffer[(x, indicator.y)].set_style(role);
        }
        buffer.set_stringn(
            indicator.x,
            indicator.y,
            text::fit_line(
                &if below > 0 {
                    format!("{below} more ↓")
                } else {
                    String::new()
                },
                indicator.width,
                crate::style::TextFlow::Truncate,
                tmt_cli_style::grid::Align::Left,
            ),
            usize::from(indicator.width),
            role,
        );
    }
    Ok(frame)
}

#[cfg(test)]
mod tests;
