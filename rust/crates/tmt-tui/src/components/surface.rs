//! Component admission lowers into the existing schema, geometry and painter.
//! Implements Full-screen interaction's overlay, key-help and scroll contracts.
use super::{ListFrame, ListRow, ListState, Modal, Placement, ScrollState, collection};
use crate::{
    Error, Kind, MarkupElement,
    binding::{self, Node, Schema, Sources},
    geometry::{self},
    paint,
    style::{Direction, Extent},
    text,
};
use ratatui::{buffer::Buffer, layout::Rect, style::Style};
use serde_json::Value;
use tmt_cli_style::{Depth, Role, Theme, table::escape};
use unicode_width::UnicodeWidthStr;

const HELP_ROWS: &str = r#"<tmt-view version="1"><tmt-repeat each="$.help.sections" as="section"><tmt-col id-bind="section.id" class="shrink-0"><tmt-text bind="section.title" token="muted" wrap="true"/><tmt-repeat each="section.entries" as="entry"><tmt-row id-bind="entry.id" row-bind="entry.id" class="shrink-0 gap-2"><tmt-text id="keys" bind="entry.keys" token="accent" wrap="true"/><tmt-text id="description" bind="entry.description" token="text" wrap="true"/></tmt-row></tmt-repeat></tmt-col></tmt-repeat></tmt-view>"#;

fn fail(file: &str, element: &MarkupElement, message: impl Into<String>) -> Error {
    Error {
        file: file.into(),
        location: element.location,
        message: message.into(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModalSlot {
    Footer,
    Status,
    Query,
}

#[derive(Clone)]
struct Spec {
    modal: Modal,
    help_ids: Vec<Vec<String>>,
    slots: Vec<ModalSlot>,
    list: Option<collection::ListSpec>,
}
impl Spec {
    fn has(&self, slot: ModalSlot) -> bool {
        self.slots.contains(&slot)
    }
}

pub struct Template<S> {
    binding: binding::Template<S>,
    position: binding::Template<S>,
    spec: Spec,
}
pub struct ModalSurface {
    node: Node,
    position: Node,
    spec: Spec,
    rows: Vec<ListRow>,
}

/// A surface is one modal or picker with one scroll body and fixed text slots.
/// Components cannot occur in repeats yet; primitive templates inside the
/// scroll remain lexical and fully bounded by the normal binding owner.
pub fn compile<A: Sources>(
    file: &str,
    parsed: &MarkupElement,
    schema: &Schema,
    sources: &A,
) -> Result<Template<A::Source>, Error> {
    if parsed.kind != Kind::View
        || parsed.children.len() != 1
        || !matches!(parsed.children[0].kind, Kind::Modal | Kind::Picker)
    {
        return Err(fail(
            file,
            parsed,
            "component surface requires one tmt-modal or tmt-picker inside tmt-view",
        ));
    }
    let mut lowered = parsed.clone();
    let element = &mut lowered.children[0];
    if element.kind == Kind::Picker {
        let mut choices = None;
        let mut query = false;
        for (index, child) in element.children.iter().enumerate() {
            if matches!(child.kind, Kind::List | Kind::Table) {
                if choices.replace(index).is_some() {
                    return Err(fail(file, child, "picker requires exactly one list/table"));
                }
            } else if child.kind == Kind::Text
                && child
                    .attributes
                    .get("slot")
                    .is_some_and(|slot| slot == "query")
            {
                query = true;
            } else if child.kind != Kind::Text {
                return Err(fail(
                    file,
                    child,
                    "picker children must be list/table and query/footer/status text",
                ));
            }
        }
        let index = choices.ok_or_else(|| fail(file, element, "picker requires one list/table"))?;
        if !query {
            return Err(fail(
                file,
                element,
                "picker requires a query slot (may be empty for selection-only)",
            ));
        }
        let child = element.children.remove(index);
        let mut body = crate::parse(
            file,
            r#"<tmt-view version="1"><tmt-scroll id="picker-body"/></tmt-view>"#,
        )?
        .children
        .remove(0);
        body.location = element.location;
        body.children.push(child);
        element.children.insert(0, body);
        element.kind = Kind::Modal;
    }
    let title = element
        .attributes
        .remove("title")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| fail(file, element, "tmt-modal requires a nonempty title"))?;
    let placement = match element
        .attributes
        .remove("placement")
        .as_deref()
        .unwrap_or("center")
    {
        "body" => Placement::Body,
        "center" => Placement::Center,
        "docked" => Placement::Docked,
        _ => {
            return Err(fail(
                file,
                element,
                "placement must be body, center or docked",
            ));
        }
    };
    let mut spec = Spec {
        modal: Modal { title, placement },
        help_ids: Vec::new(),
        slots: Vec::new(),
        list: None,
    };
    let mut scrolls = 0;
    for child in &mut element.children {
        match child.kind {
            Kind::Scroll => scrolls += 1,
            Kind::Text => {
                let slot = child.attributes.remove("slot").ok_or_else(|| {
                    fail(file, child, "modal text requires a footer or status slot")
                })?;
                let kind = match slot.as_str() {
                    "footer" => ModalSlot::Footer,
                    "status" => ModalSlot::Status,
                    "query" => ModalSlot::Query,
                    _ => return Err(fail(file, child, "slot must be query, footer or status")),
                };
                if spec.has(kind) {
                    return Err(fail(file, child, format!("duplicate {slot} slot")));
                }
                spec.slots.push(kind);
            }
            _ => {
                return Err(fail(
                    file,
                    child,
                    "modal children must be one tmt-scroll and footer/status text",
                ));
            }
        }
    }
    if scrolls != 1 {
        return Err(fail(file, element, "modal requires exactly one tmt-scroll"));
    }
    // Canonical slot order makes runtime ownership explicit regardless of authored order.
    element.children.sort_by_key(|child| match child.kind {
        Kind::Scroll => 0,
        _ => 1,
    });
    let mut ids = Vec::new();
    lower(
        file,
        &mut lowered,
        &mut ids,
        false,
        &mut spec,
        &mut [0, 0],
        schema,
    )?;
    bounds(file, &lowered, 0, &mut 0)?;
    let position = crate::parse(
        file,
        r#"<tmt-view version="1"><tmt-text token="muted" class="truncate"/></tmt-view>"#,
    )?;
    Ok(Template {
        binding: binding::compile(file, &lowered, schema, sources)?,
        position: binding::compile(
            file,
            &position,
            &Schema::Object(Default::default()),
            sources,
        )?,
        spec,
    })
}

pub(crate) fn bounds(
    file: &str,
    element: &MarkupElement,
    depth: usize,
    nodes: &mut u32,
) -> Result<(), Error> {
    *nodes += 1;
    if depth >= crate::MAX_DEPTH || *nodes > crate::MAX_NODES {
        return Err(fail(
            file,
            element,
            "lowered component template exceeds markup depth/node limits",
        ));
    }
    for child in &element.children {
        bounds(file, child, depth + 1, nodes)?;
    }
    Ok(())
}

fn lower(
    file: &str,
    element: &mut MarkupElement,
    ids: &mut Vec<String>,
    dynamic: bool,
    spec: &mut Spec,
    counts: &mut [usize; 2],
    schema: &Schema,
) -> Result<(), Error> {
    let dynamic =
        dynamic || element.kind == Kind::Repeat || element.attributes.contains_key("id-bind");
    let component = matches!(
        element.kind,
        Kind::Modal | Kind::Scroll | Kind::KeyHelp | Kind::List | Kind::Table
    );
    if component && (dynamic || !element.attributes.contains_key("id")) {
        return Err(fail(
            file,
            element,
            "components require literal stable IDs outside repeat/dynamic ID scopes",
        ));
    }
    let length = ids.len();
    if let Some(id) = element.attributes.get("id") {
        ids.push(id.clone());
    }
    match element.kind {
        Kind::Modal | Kind::Scroll => {
            let index = usize::from(element.kind == Kind::Scroll);
            counts[index] += 1;
            if counts[index] > 1 {
                return Err(fail(
                    file,
                    element,
                    "nested modal/scroll components are not supported",
                ));
            }
            element.kind = Kind::Col;
            element.style.direction = Direction::Column;
        }
        Kind::List | Kind::Table => {
            if spec.list.is_some() {
                return Err(fail(
                    file,
                    element,
                    "one scroll surface supports one list/table",
                ));
            }
            spec.list = Some(collection::lower(file, element, ids, schema)?);
        }
        Kind::Picker => return Err(fail(file, element, "picker must be the surface root")),
        Kind::KeyHelp => {
            if !element.children.is_empty() {
                return Err(fail(
                    file,
                    element,
                    "tmt-key-help binds its model and has no children",
                ));
            }
            let path = element
                .attributes
                .remove("bind")
                .ok_or_else(|| fail(file, element, "tmt-key-help requires bind"))?;
            let heading_token = element
                .attributes
                .remove("heading-token")
                .map(|value| {
                    tmt_cli_style::Role::parse(&value).ok_or_else(|| {
                        fail(
                            file,
                            element,
                            "heading-token requires an existing theme role",
                        )
                    })
                })
                .transpose()?
                .unwrap_or(tmt_cli_style::Role::Muted);
            let heading_bold = match element.attributes.remove("heading-bold").as_deref() {
                None | Some("false") => false,
                Some("true") => true,
                _ => return Err(fail(file, element, "heading-bold must be true or false")),
            };
            let section_gap = element
                .attributes
                .remove("section-gap")
                .map(|value| {
                    value
                        .parse::<u16>()
                        .ok()
                        .filter(|n| {
                            *n <= crate::style::MAX_CELLS
                                && value.bytes().all(|b| b.is_ascii_digit())
                        })
                        .ok_or_else(|| {
                            fail(
                                file,
                                element,
                                "section-gap requires an integer from 0 to 4096",
                            )
                        })
                })
                .transpose()?
                .unwrap_or(element.style.gap[1]);
            let mut prototype = crate::parse(file, HELP_ROWS)?;
            let heading = &mut prototype.children[0].children[0].children[0];
            heading.style.token = Some(heading_token);
            heading.style.bold = heading_bold;
            element.style.gap[1] = section_gap;
            prototype.children[0]
                .attributes
                .insert("each".into(), format!("{path}.sections"));
            // Generated syntax carries the authored component's location.
            fn locate(element: &mut MarkupElement, location: crate::Location) {
                element.location = location;
                for child in &mut element.children {
                    locate(child, location);
                }
            }
            locate(&mut prototype, element.location);
            element.kind = Kind::Col;
            element.style.direction = Direction::Column;
            element.children = prototype.children;
            spec.help_ids.push(ids.clone());
        }
        _ => {}
    }
    if element.attributes.contains_key("slot") {
        return Err(fail(
            file,
            element,
            "slots belong only to direct modal footer/status text",
        ));
    }
    for child in &mut element.children {
        lower(file, child, ids, dynamic, spec, counts, schema)?;
    }
    ids.truncate(length);
    Ok(())
}

impl<S> Template<S> {
    pub fn materialize<A: Sources<Source = S>>(
        &self,
        file: &str,
        data: &Value,
        sources: &A,
    ) -> Result<ModalSurface, Error> {
        let mut node = self.binding.materialize(file, data, sources)?;
        let rows = if let Some(spec) = &self.spec.list {
            collection::extract(
                file,
                &mut node,
                spec,
                crate::Location { line: 1, column: 1 },
            )?
        } else {
            Vec::new()
        };
        Ok(ModalSurface {
            node,
            rows,
            position: self
                .position
                .materialize(file, &serde_json::json!({}), sources)?
                .children
                .remove(0),
            spec: self.spec.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub rect: Rect,
    pub id: Vec<String>,
    pub row_id: Option<String>,
}
#[derive(Debug)]
pub struct FrameMap {
    pub areas: super::ModalAreas,
    pub hits: Vec<Hit>,
    pub list: Option<ListFrame>,
    pub query: Rect,
}
pub struct RenderStyle<'a> {
    pub theme: &'a Theme,
    pub depth: Depth,
}
impl FrameMap {
    pub fn hit_at(&self, x: u16, y: u16) -> Option<&Hit> {
        self.hits
            .iter()
            .rev()
            .find(|hit| hit.rect.contains((x, y).into()))
    }
}

fn help_width(node: &Node, ids: &[Vec<String>]) -> usize {
    fn labels(node: &Node) -> usize {
        let own = node
            .id
            .as_ref()
            .filter(|id| id.last().is_some_and(|id| id == "keys"))
            .and(node.text.as_deref())
            .map_or(0, |text| escape(text).width());
        node.children.iter().map(labels).fold(own, usize::max)
    }
    if node.id.as_ref().is_some_and(|id| ids.contains(id)) {
        labels(node)
    } else {
        node.children
            .iter()
            .map(|child| help_width(child, ids))
            .max()
            .unwrap_or(0)
    }
}

fn prepare(node: &mut Node, ids: &[Vec<String>], width: u16, in_help: bool, key_width: usize) {
    let in_help = in_help || node.id.as_ref().is_some_and(|id| ids.contains(id));
    let width = match node.style.width {
        Extent::Cells(cells) => cells,
        _ => width,
    }
    .min(node.style.max_width.unwrap_or(u16::MAX))
    .saturating_sub(node.style.padding[0].saturating_mul(2));
    // A scroll body has intrinsic visual-line height, never flex shrinking.
    node.style.shrink = 0;
    node.style.grow = 0;
    if in_help && node.row_id.is_some() && node.children.len() == 2 {
        let stacked = usize::from(width).saturating_sub(key_width.saturating_add(2)) < 20;
        node.style.direction = if stacked {
            Direction::Column
        } else {
            Direction::Row
        };
        node.style.gap = if stacked { [0, 0] } else { [2, 0] };
        node.children[0].style.width = Extent::Cells(if stacked {
            width
        } else {
            key_width.min(usize::from(u16::MAX)) as u16
        });
        node.children[1].style.width = Extent::Cells(if stacked {
            width
        } else {
            width.saturating_sub(key_width as u16).saturating_sub(2)
        });
        node.children[1].style.min_width = Some(0);
        if stacked {
            node.children[1].style.width = Extent::Cells(width);
            node.children[1].style.padding[0] = u16::from(width > 2);
        }
    }
    for child in &mut node.children {
        prepare(child, ids, width, in_help, key_width);
    }
}

fn paint_node(
    node: &Node,
    area: Rect,
    buffer: &mut Buffer,
    theme: &Theme,
    depth: Depth,
    selection: &mut impl FnMut(Role) -> Style,
) -> Result<(), String> {
    if area.is_empty() {
        return Ok(());
    }
    let mut cells = geometry::layout(node, [area.width, area.height], text::measure)?;
    collection::translate(&mut cells, area, 0);
    paint::paint(&cells, buffer, theme, depth, selection);
    Ok(())
}

/// Render one modal surface into the caller's buffer. The caller may keep scene
/// and scroll behind one RefCell to retain an existing `render(&App)` interface.
/// All body geometry and text painting use the existing markup owners.
pub fn render(
    scene: &ModalSurface,
    scroll: &mut ScrollState,
    body: Rect,
    buffer: &mut Buffer,
    style: RenderStyle<'_>,
    mut selection: impl FnMut(Role) -> Style,
) -> Result<FrameMap, String> {
    if scene.spec.list.is_some() {
        return Err("list/table surfaces require render_list and caller ListState".into());
    }
    render_inner(scene, scroll, body, buffer, style, &mut selection, None)
}

pub fn render_list(
    scene: &ModalSurface,
    state: &mut ListState,
    body: Rect,
    buffer: &mut Buffer,
    style: RenderStyle<'_>,
    mut selection: impl FnMut(Role) -> Style,
) -> Result<FrameMap, String> {
    if scene.spec.list.is_none() {
        return Err("render_list requires a list/table surface".into());
    }
    state.reconcile(scene.rows.clone())?;
    let selected = state.selected().map(str::to_owned);
    render_inner(
        scene,
        &mut state.scroll,
        body,
        buffer,
        style,
        &mut selection,
        Some(selected.as_deref()),
    )
}

fn render_inner(
    scene: &ModalSurface,
    scroll: &mut ScrollState,
    body: Rect,
    buffer: &mut Buffer,
    style: RenderStyle<'_>,
    selection: &mut impl FnMut(Role) -> Style,
    selected: Option<Option<&str>>,
) -> Result<FrameMap, String> {
    let RenderStyle { theme, depth } = style;
    let modal_node = &scene.node.children[0];
    let mut content = modal_node.children[0].clone();
    content.style.token = content
        .style
        .token
        .or(modal_node.style.token)
        .or(scene.node.style.token);
    content.selected |= modal_node.selected || scene.node.selected;
    fn intrinsic(node: &Node) -> usize {
        let own = node.text.as_deref().map_or(0, |value| {
            usize::from(text::measure(value, node.style.text_flow, geometry::Space::MaxContent)[0])
        });
        node.children.iter().map(intrinsic).fold(own, usize::max)
    }
    let demand_width = match modal_node.style.width {
        Extent::Cells(width) => width,
        _ => intrinsic(&content)
            .saturating_add(help_width(&content, &scene.spec.help_ids))
            .saturating_add(6)
            .min(usize::from(u16::MAX)) as u16,
    };
    if let Some(spec) = &scene.spec.list {
        collection::select(&mut content, spec, selected.flatten());
    }
    let demand_height = body.height;
    let mut areas = scene.spec.modal.areas(
        body.intersection(buffer.area),
        [demand_width, demand_height],
        scene.spec.has(ModalSlot::Footer),
        scene.spec.has(ModalSlot::Status),
    );
    if scene.spec.has(ModalSlot::Query) {
        areas.content.y = areas
            .content
            .y
            .saturating_add(u16::from(areas.content.height > 0));
        areas.content.height = areas.content.height.saturating_sub(1);
    }
    let key_width = help_width(&content, &scene.spec.help_ids);
    prepare(
        &mut content,
        &scene.spec.help_ids,
        areas.content.width,
        false,
        key_width,
    );
    // The logical scroll surface is bounded; only visible cells get painted.
    content.style.height = Extent::Cells(u16::MAX);
    let mut cells = geometry::layout(&content, [areas.content.width, u16::MAX], text::measure)?;
    let rows = cells
        .iter()
        .skip(1)
        .map(|cell| i64::from(cell.rect.y) + i64::from(cell.rect.height))
        .max()
        .unwrap_or(0)
        .max(0) as usize;
    if rows > usize::from(u16::MAX) {
        return Err("scroll content exceeds 65535 visual lines".into());
    }
    let demand_height = rows
        .saturating_add(
            3 + usize::from(scene.spec.has(ModalSlot::Footer))
                + usize::from(scene.spec.has(ModalSlot::Status))
                + usize::from(scene.spec.has(ModalSlot::Query)),
        )
        .min(usize::from(u16::MAX)) as u16;
    let mut areas = scene.spec.modal.areas(
        body.intersection(buffer.area),
        [demand_width, demand_height],
        scene.spec.has(ModalSlot::Footer),
        scene.spec.has(ModalSlot::Status),
    );
    let query = if scene.spec.has(ModalSlot::Query) {
        let query = Rect {
            height: u16::from(areas.content.height > 0),
            ..areas.content
        };
        areas.content.y = areas.content.y.saturating_add(query.height);
        areas.content.height = areas.content.height.saturating_sub(query.height);
        query
    } else {
        Rect::default()
    };
    let list = if selected.is_some() {
        Some(collection::publish(
            &cells,
            &scene.spec.list.as_ref().expect("list render").id,
            scroll,
            &scene.rows,
            selected.flatten(),
            areas.content,
            rows,
        ))
    } else {
        scroll.update(areas.content, rows, None);
        None
    };
    collection::translate(&mut cells, areas.content, scroll.offset());
    scene.spec.modal.paint(areas, buffer, theme, depth);
    let hits = paint::paint(&cells, buffer, theme, depth, &mut *selection)
        .into_iter()
        .filter_map(|hit| {
            hit.id.map(|id| Hit {
                rect: Rect::new(
                    hit.rect.x as u16,
                    hit.rect.y as u16,
                    hit.rect.width as u16,
                    hit.rect.height as u16,
                ),
                id: id.to_vec(),
                row_id: hit.row_id.map(str::to_owned),
            })
        })
        .collect();
    for (child, slot) in modal_node.children.iter().skip(1).zip(&scene.spec.slots) {
        let area = match slot {
            ModalSlot::Footer => areas.footer,
            ModalSlot::Status => areas.status,
            ModalSlot::Query => query,
        };
        paint_node(child, area, buffer, theme, depth, &mut *selection)?;
    }
    let mut position = scene.position.clone();
    position.text = Some(scroll.position());
    paint_node(
        &position,
        areas.position,
        buffer,
        theme,
        depth,
        &mut *selection,
    )?;
    Ok(FrameMap {
        areas,
        hits,
        list,
        query,
    })
}

#[cfg(test)]
mod tests;
