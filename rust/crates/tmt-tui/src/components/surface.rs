//! Component admission lowers into the existing schema, geometry and painter.
//! Implements Full-screen interaction's overlay, key-help and scroll contracts.
use super::{Modal, Placement, ScrollState};
use crate::{
    Error, Kind, MarkupElement,
    binding::{self, Node, Schema, Sources},
    geometry::{self, Cell},
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
}

#[derive(Clone)]
struct Spec {
    modal: Modal,
    help_ids: Vec<Vec<String>>,
    slots: Vec<ModalSlot>,
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
}

/// A surface is one modal with one scroll body and optional footer/status text.
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
        || parsed.children[0].kind != Kind::Modal
    {
        return Err(fail(
            file,
            parsed,
            "component surface requires exactly one tmt-modal inside tmt-view",
        ));
    }
    let mut lowered = parsed.clone();
    let element = &mut lowered.children[0];
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
                    _ => return Err(fail(file, child, "slot must be footer or status")),
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
    lower(file, &mut lowered, &mut ids, false, &mut spec, &mut [0, 0])?;
    fn bounds(
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

fn lower(
    file: &str,
    element: &mut MarkupElement,
    ids: &mut Vec<String>,
    dynamic: bool,
    spec: &mut Spec,
    counts: &mut [usize; 2],
) -> Result<(), Error> {
    let dynamic =
        dynamic || element.kind == Kind::Repeat || element.attributes.contains_key("id-bind");
    let component = matches!(element.kind, Kind::Modal | Kind::Scroll | Kind::KeyHelp);
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
            let mut prototype = crate::parse(file, HELP_ROWS)?;
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
        lower(file, child, ids, dynamic, spec, counts)?;
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
        Ok(ModalSurface {
            node: self.binding.materialize(file, data, sources)?,
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

fn translate(cells: &mut [Cell<'_>], area: Rect, offset: usize) {
    let dx = i32::from(area.x);
    let dy = i32::from(area.y).saturating_sub(offset.min(i32::MAX as usize) as i32);
    let clip = geometry::Rect {
        x: i32::from(area.x),
        y: i32::from(area.y),
        width: u32::from(area.width),
        height: u32::from(area.height),
    };
    for cell in cells {
        for rect in [&mut cell.rect, &mut cell.content, &mut cell.clip] {
            rect.x = rect.x.saturating_add(dx);
            rect.y = rect.y.saturating_add(dy);
        }
        cell.clip = cell.clip.intersect(clip);
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
    translate(&mut cells, area, 0);
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
    let demand_height = body.height;
    let areas = scene.spec.modal.areas(
        body.intersection(buffer.area),
        [demand_width, demand_height],
        scene.spec.has(ModalSlot::Footer),
        scene.spec.has(ModalSlot::Status),
    );
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
                + usize::from(scene.spec.has(ModalSlot::Status)),
        )
        .min(usize::from(u16::MAX)) as u16;
    let areas = scene.spec.modal.areas(
        body.intersection(buffer.area),
        [demand_width, demand_height],
        scene.spec.has(ModalSlot::Footer),
        scene.spec.has(ModalSlot::Status),
    );
    scroll.update(areas.content, rows, None);
    translate(&mut cells, areas.content, scroll.offset());
    scene.spec.modal.paint(areas, buffer, theme, depth);
    let hits = paint::paint(&cells, buffer, theme, depth, &mut selection)
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
        let area = if *slot == ModalSlot::Footer {
            areas.footer
        } else {
            areas.status
        };
        paint_node(child, area, buffer, theme, depth, &mut selection)?;
    }
    let mut position = scene.position.clone();
    position.text = Some(scroll.position());
    paint_node(
        &position,
        areas.position,
        buffer,
        theme,
        depth,
        &mut selection,
    )?;
    Ok(FrameMap { areas, hits })
}

#[cfg(test)]
mod tests;
