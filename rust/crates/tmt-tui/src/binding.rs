//! Schema admission precedes bounded expansion; neither stage acquires data.
use crate::{Error, Kind, MAX_NODES, MarkupElement, style::CellStyle};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use tmt_cli_style::theme::Role;

pub const MAX_EXPANDED_NODES: usize = MAX_NODES as usize;
pub const MAX_REPEAT_WORK: usize = 20_000;
pub const MAX_BOUND_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_ID_BYTES: usize = 256;

/// StableId promises application identity, never a collection position.
#[derive(Debug, Clone)]
pub enum Schema {
    Scalar,
    Boolean,
    StableId,
    Object(BTreeMap<String, Schema>),
    Collection(Box<Schema>),
}
#[derive(Clone, Copy)]
enum Shape {
    Scalar,
    Boolean,
    StableId,
    Object,
    Collection,
}
impl Schema {
    fn shape(&self) -> Shape {
        match self {
            Self::Scalar => Shape::Scalar,
            Self::Boolean => Shape::Boolean,
            Self::StableId => Shape::StableId,
            Self::Object(_) => Shape::Object,
            Self::Collection(_) => Shape::Collection,
        }
    }
}
impl Shape {
    fn accepts(self, value: &Value) -> bool {
        match self {
            Self::Scalar => {
                value.is_null() || value.is_string() || value.is_boolean() || value.is_number()
            }
            Self::Boolean => value.is_boolean(),
            Self::StableId => value.as_str().is_some_and(stable_id),
            Self::Object => value.is_object(),
            Self::Collection => value.is_array(),
        }
    }
}
pub type Scopes<'a> = BTreeMap<String, &'a Value>;
pub type Schemas<'a> = BTreeMap<String, &'a Schema>;

/// Application-owned source/format registry and in-memory evaluation.
/// Callbacks acquire no data; their allocation is the application's responsibility.
pub trait Sources {
    type Source;
    fn compile(
        &self,
        from: &str,
        format: &str,
        scopes: &Schemas<'_>,
    ) -> Result<Self::Source, String>;
    fn resolve(&self, source: &Self::Source, scopes: &Scopes<'_>)
    -> Result<Option<String>, String>;
}

/// What a width-conditional branch measures. Container is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Of {
    /// The content width of the switch's parent box.
    Container,
    /// The viewport width given to `geometry::layout`.
    Terminal,
}

/// The condition a `Switch`, `Case` or `Default` node carries. Geometry resolves
/// switches before laying out, so no condition reaches a painted cell.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Cond {
    #[default]
    None,
    Switch(Of),
    /// The branch applies from this width, in cells, up.
    Case(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub kind: Kind,
    pub cond: Cond,
    pub style: CellStyle,
    /// Components preserve occurrence identity without delimiter collisions.
    pub id: Option<Vec<String>>,
    pub row_id: Option<String>,
    pub selected: bool,
    /// Null is None; a missing required bind is an error, never null.
    pub text: Option<String>,
    pub children: Vec<Node>,
}

#[derive(Debug)]
struct Path(Vec<String>);
impl Path {
    fn parse(text: &str) -> Result<Self, String> {
        let parts: Vec<_> = text.split('.').map(str::to_owned).collect();
        if parts.is_empty()
            || parts
                .iter()
                .enumerate()
                .any(|(i, p)| !(i == 0 && p == "$") && !identifier(p))
        {
            return Err(
                "requires a root or lexical dotted path, without indexing or expressions".into(),
            );
        }
        Ok(Self(parts))
    }
    fn schema<'a>(&self, scopes: &Schemas<'a>) -> Option<&'a Schema> {
        let mut value = *scopes.get(&self.0[0])?;
        for key in &self.0[1..] {
            let Schema::Object(fields) = value else {
                return None;
            };
            value = fields.get(key)?;
        }
        Some(value)
    }
    fn value<'a>(&self, scopes: &Scopes<'a>) -> Option<&'a Value> {
        let mut value = *scopes.get(&self.0[0])?;
        for key in &self.0[1..] {
            value = value.as_object()?.get(key)?;
        }
        Some(value)
    }
}
/// Component models are outside dynamic scopes; normal path syntax still has
/// one owner. Return the declared type for a root-only component model path.
pub(crate) fn root_schema<'a>(schema: &'a Schema, path: &str) -> Result<&'a Schema, String> {
    Path::parse(path)?
        .schema(&BTreeMap::from([("$".into(), schema)]))
        .ok_or_else(|| "unknown root model path".into())
}
fn identifier(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
pub(crate) fn stable_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_ID_BYTES
        && text.trim() == text
        && !text.chars().any(char::is_control)
        && !text.bytes().all(|b| b.is_ascii_digit())
}

/// Opaque compiled handles retain locations for runtime admission errors.
pub struct Template<S> {
    element: MarkupElement,
    paths: BTreeMap<String, (Path, Shape)>,
    root: Shape,
    repeat: Option<(Path, String, Shape)>,
    source: Option<S>,
    children: Vec<Template<S>>,
}
fn fail(file: &str, element: &MarkupElement, message: impl Into<String>) -> Error {
    let tag = match element.kind {
        Kind::View => "tmt-view",
        Kind::Row => "tmt-row",
        Kind::Col => "tmt-col",
        Kind::Cell => "tmt-cell",
        Kind::Text => "tmt-text",
        Kind::Repeat => "tmt-repeat",
        Kind::Modal => "tmt-modal",
        Kind::Scroll => "tmt-scroll",
        Kind::KeyHelp => "tmt-key-help",
        Kind::List => "tmt-list",
        Kind::Table => "tmt-table",
        Kind::Picker => "tmt-picker",
        Kind::Switch => "tmt-switch",
        Kind::Case => "tmt-case",
        Kind::Default => "tmt-default",
    };
    Error {
        file: file.into(),
        location: element.location,
        message: format!("<{tag}>: {}", message.into()),
    }
}

/// Compile a structurally admitted template against application-owned schemas.
pub fn compile<A: Sources>(
    file: &str,
    element: &MarkupElement,
    schema: &Schema,
    sources: &A,
) -> Result<Template<A::Source>, Error> {
    if element.kind != Kind::View {
        return Err(fail(file, element, "binding root must be tmt-view"));
    }
    check(
        file,
        element,
        &BTreeMap::from([("$".into(), schema)]),
        sources,
    )
}
fn check<A: Sources>(
    file: &str,
    element: &MarkupElement,
    scopes: &Schemas<'_>,
    sources: &A,
) -> Result<Template<A::Source>, Error> {
    // parse guarantees repeat keys; binding paths below index only present attributes.
    let attrs = &element.attributes;
    let error = |message| fail(file, element, message);
    if matches!(
        element.kind,
        Kind::Modal | Kind::Scroll | Kind::KeyHelp | Kind::List | Kind::Table | Kind::Picker
    ) || attrs.contains_key("slot")
    {
        return Err(error(
            "component markup must be lowered by components::surface::compile".into(),
        ));
    }
    let path = |key: &str| -> Result<Path, Error> {
        Path::parse(&attrs[key]).map_err(|why| error(format!("{key}={:?}: {why}", attrs[key])))
    };
    let mut inner = scopes.clone();
    let repeat = if element.kind == Kind::Repeat {
        let each = path("each")?;
        let Some(Schema::Collection(item)) = each.schema(scopes) else {
            return Err(error(format!(
                "each={:?}: expected a declared collection",
                attrs["each"]
            )));
        };
        let alias = &attrs["as"];
        if !identifier(alias) {
            return Err(error(format!("as={alias:?}: invalid lexical name")));
        }
        inner.insert(alias.clone(), item);
        Some((each, alias.clone(), item.shape()))
    } else {
        None
    };
    let mut paths = BTreeMap::new();
    for key in ["bind", "id-bind", "row-bind", "token-bind"] {
        if attrs.contains_key(key) {
            let compiled = path(key)?;
            let valid = matches!(
                (key, compiled.schema(scopes)),
                ("id-bind" | "row-bind", Some(Schema::StableId))
                    | (
                        "bind",
                        Some(Schema::Scalar | Schema::Boolean | Schema::StableId)
                    )
                    | ("token-bind", Some(Schema::Scalar | Schema::StableId))
            );
            if !valid {
                return Err(error(format!(
                    "{key}={:?}: unknown path or incompatible schema (IDs require StableId)",
                    attrs[key]
                )));
            }
            let shape = compiled.schema(scopes).expect("checked path").shape();
            paths.insert(key.into(), (compiled, shape));
        }
    }
    for key in ["id", "row-id"] {
        if let Some(value) = attrs.get(key)
            && !stable_id(value)
        {
            return Err(error(format!(
                "{key}={value:?}: requires a nonempty stable ID of at most {MAX_ID_BYTES} bytes"
            )));
        }
    }
    if (attrs.contains_key("row-id") || attrs.contains_key("row-bind"))
        && !(attrs.contains_key("id") || attrs.contains_key("id-bind"))
    {
        return Err(error("actionable row requires id or id-bind".into()));
    }
    if attrs.contains_key("token") && attrs.contains_key("token-bind") {
        return Err(error("use either token or token-bind".into()));
    }
    if let Some(value) = attrs.get("selected")
        && !["true", "false"].contains(&value.as_str())
    {
        return Err(error(format!("selected={value:?}: expected true or false")));
    }
    let source = if let Some(from) = attrs.get("from") {
        Some(
            sources
                .compile(
                    from,
                    attrs.get("format").map_or("text", String::as_str),
                    scopes,
                )
                .map_err(|why| {
                    error(format!(
                        "from={from:?} format={:?}: {why}",
                        attrs.get("format")
                    ))
                })?,
        )
    } else {
        if attrs.contains_key("format") {
            return Err(error(
                "format requires from; bound fields are already display values".into(),
            ));
        }
        None
    };
    let children = element
        .children
        .iter()
        .map(|child| check(file, child, &inner, sources))
        .collect::<Result<_, _>>()?;
    let admitted = MarkupElement {
        kind: element.kind,
        style: element.style.clone(),
        attributes: attrs.clone(),
        text: element.text.clone(),
        children: Vec::new(),
        location: element.location,
    };
    Ok(Template {
        element: admitted,
        paths,
        root: scopes["$"].shape(),
        repeat,
        source,
        children,
    })
}

#[derive(Default)]
struct Budget {
    nodes: usize,
    repeats: usize,
    bytes: usize,
    ids: BTreeSet<Vec<String>>,
}
impl Budget {
    fn charge(&mut self, bytes: usize) -> Result<(), String> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or("bound text/IDs exceed 8 MiB")?;
        if self.bytes > MAX_BOUND_BYTES {
            return Err("bound text/IDs exceed 8 MiB".into());
        }
        Ok(())
    }
}
impl<S> Template<S> {
    /// The file label is supplied again; the admitted template's source positions survive.
    pub fn materialize<A: Sources<Source = S>>(
        &self,
        file: &str,
        data: &Value,
        sources: &A,
    ) -> Result<Node, Error> {
        if !self.root.accepts(data) {
            return Err(fail(file, &self.element, "root data does not match schema"));
        }
        let mut nodes = self.expand(
            file,
            &BTreeMap::from([("$".into(), data)]),
            sources,
            &[],
            &mut Budget::default(),
        )?;
        Ok(nodes.remove(0))
    }
    fn expand<A: Sources<Source = S>>(
        &self,
        file: &str,
        scopes: &Scopes<'_>,
        sources: &A,
        parent: &[String],
        budget: &mut Budget,
    ) -> Result<Vec<Node>, Error> {
        let error = |message| fail(file, &self.element, message);
        if let Some((path, alias, shape)) = &self.repeat {
            let items = path
                .value(scopes)
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    error(format!(
                        "each={:?}: missing or non-collection value",
                        self.element.attributes["each"]
                    ))
                })?;
            let mut result = Vec::new();
            for item in items {
                budget.repeats += 1;
                if budget.repeats > MAX_REPEAT_WORK {
                    return Err(error("repeat work exceeds 20,000".into()));
                }
                if !shape.accepts(item) {
                    return Err(error(format!(
                        "each={:?}: item does not match schema",
                        self.element.attributes["each"]
                    )));
                }
                let mut inner = scopes.clone();
                inner.insert(alias.clone(), item);
                for child in &self.children {
                    result.extend(child.expand(file, &inner, sources, parent, budget)?);
                }
            }
            return Ok(result);
        }
        budget.nodes += 1;
        if budget.nodes > MAX_EXPANDED_NODES {
            return Err(error("expanded nodes exceed 20,000".into()));
        }
        let bound = |key: &str| -> Result<Option<Cow<'_, str>>, Error> {
            let (path, shape) = &self.paths[key];
            let value = path.value(scopes).ok_or_else(|| {
                error(format!(
                    "{key}={:?}: missing path",
                    self.element.attributes[key]
                ))
            })?;
            if !shape.accepts(value) {
                return Err(error(format!(
                    "{key}={:?}: value does not match schema",
                    self.element.attributes[key]
                )));
            }
            match value {
                Value::Null => Ok(None),
                Value::String(text) => Ok(Some(Cow::Borrowed(text))),
                Value::Bool(value) if key == "bind" => {
                    Ok(Some(Cow::Borrowed(if *value { "true" } else { "false" })))
                }
                Value::Number(value) if key == "bind" => Ok(Some(Cow::Borrowed(value.as_str()))),
                _ => Err(error(format!(
                    "{key}={:?}: incompatible scalar value",
                    self.element.attributes[key]
                ))),
            }
        };
        let id = |literal: &str, binding: &str| -> Result<Option<Cow<'_, str>>, Error> {
            let value = if self.paths.contains_key(binding) {
                bound(binding)?
            } else {
                self.element
                    .attributes
                    .get(literal)
                    .map(|v| Cow::Borrowed(v.as_str()))
            };
            if (self.paths.contains_key(binding) || self.element.attributes.contains_key(literal))
                && !value.as_deref().is_some_and(stable_id)
            {
                return Err(error(format!("{binding}/{literal}: null or unstable ID")));
            }
            Ok(value)
        };
        let own = id("id", "id-bind")?;
        let mut scope = parent.to_vec();
        if let Some(own) = own.as_ref() {
            // Both output and duplicate-ID registry retain scoped components.
            budget
                .charge(2 * (parent.iter().map(String::len).sum::<usize>() + own.len()))
                .map_err(error)?;
            scope.push(own.to_string());
        }
        let resolved_id = own.map(|_| scope.clone());
        if let Some(id) = &resolved_id
            && !budget.ids.insert(id.clone())
        {
            return Err(error(format!("duplicate resolved id {id:?}")));
        }
        let row_id = id("row-id", "row-bind")?;
        let text = if self.paths.contains_key("bind") {
            bound("bind")?
        } else if let Some(source) = &self.source {
            sources
                .resolve(source, scopes)
                .map_err(|why| error(format!("from={:?}: {why}", self.element.attributes["from"])))?
                .map(Cow::Owned)
        } else {
            Some(Cow::Borrowed(self.element.text.as_str()))
        };
        budget
            .charge(text.as_ref().map_or(0, |v| v.len()) + row_id.as_ref().map_or(0, |v| v.len()))
            .map_err(error)?;
        let text = text.map(Cow::into_owned);
        let row_id = row_id.map(Cow::into_owned);
        let mut style = self.element.style.clone();
        if self.paths.contains_key("token-bind") {
            style.token = bound("token-bind")?
                .map(|token| {
                    Role::parse(&token).ok_or_else(|| {
                        error(format!(
                            "token-bind={:?}: unknown token",
                            self.element.attributes["token-bind"]
                        ))
                    })
                })
                .transpose()?;
        }
        let mut children = Vec::new();
        if self.element.kind == Kind::Switch {
            // Only one branch is ever laid out, so branches may reuse an ID:
            // a cell present at several widths keeps its identity across a
            // resize. IDs still must not collide with anything outside the switch.
            let outside = budget.ids.clone();
            let mut seen = outside.clone();
            for child in &self.children {
                budget.ids.clone_from(&outside);
                children.extend(child.expand(file, scopes, sources, &scope, budget)?);
                seen.append(&mut budget.ids);
            }
            budget.ids = seen;
        } else {
            for child in &self.children {
                children.extend(child.expand(file, scopes, sources, &scope, budget)?);
            }
        }
        let cond = match self.element.kind {
            Kind::Switch => Cond::Switch(
                match self.element.attributes.get("of").map(String::as_str) {
                    Some("terminal") => Of::Terminal,
                    _ => Of::Container,
                },
            ),
            Kind::Case => Cond::Case(
                tmt_cli_style::breakpoint::by_name(&self.element.attributes["min"])
                    .expect("admitted min")
                    .cells,
            ),
            _ => Cond::None,
        };
        Ok(vec![Node {
            kind: self.element.kind,
            cond,
            style,
            id: resolved_id,
            row_id,
            selected: self
                .element
                .attributes
                .get("selected")
                .is_some_and(|v| v == "true"),
            text,
            children,
        }])
    }
}

#[cfg(test)]
mod tests;
