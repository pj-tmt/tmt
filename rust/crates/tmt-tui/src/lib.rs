//! Internal TMT markup admission. Templates have checked structure and static
//! styles and binding are checked separately, before geometry and buffer painting.

use std::{collections::BTreeMap, fmt};

pub mod app;
pub mod binding;
pub mod components;
pub mod geometry;
pub mod paint;
pub mod style;
pub mod text;

pub const MAX_BYTES: usize = 256 * 1024;
pub const MAX_DEPTH: usize = 32;
pub const MAX_NODES: u32 = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    View,
    Row,
    Col,
    Cell,
    Text,
    Repeat,
    Modal,
    Scroll,
    KeyHelp,
    List,
    Table,
    Picker,
    /// Width-conditional layout: exactly one `Case`/`Default` branch is laid out.
    Switch,
    Case,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkupElement {
    pub kind: Kind,
    pub style: style::CellStyle,
    pub attributes: BTreeMap<String, String>,
    pub text: String,
    pub children: Vec<MarkupElement>,
    pub location: Location,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub file: String,
    pub location: Location,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.file, self.location.line, self.location.column, self.message
        )
    }
}
impl std::error::Error for Error {}

fn error(file: &str, pos: roxmltree::TextPos, message: impl Into<String>) -> Error {
    Error {
        file: file.into(),
        location: Location {
            line: pos.row,
            column: pos.col,
        },
        message: message.into(),
    }
}

/// Admit version 1 structure, including templates that may later repeat zero
/// times. No I/O occurs; callers must bound file acquisition independently.
/// Classes, wrap and literal theme tokens are compiled to cell styles. Paths,
/// formats, dynamic tokens, IDs and row-track attributes remain uncompiled.
pub fn parse(file: &str, source: &str) -> Result<MarkupElement, Error> {
    if source.len() > MAX_BYTES {
        return Err(error(
            file,
            roxmltree::TextPos::new(1, 1),
            "layout exceeds 256 KiB",
        ));
    }
    check_depth(file, source)?;
    let doc = roxmltree::Document::parse_with_options(
        source,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_NODES,
        },
    )
    .map_err(|e| error(file, e.pos(), e.to_string()))?;
    let root = doc.root_element();
    if root.tag_name().name() != "tmt-view" || root.attribute("version") != Some("1") {
        return Err(error(
            file,
            doc.text_pos_at(root.range().start),
            "root must be <tmt-view version=\"1\">",
        ));
    }
    read(file, root, true, &mut (0, roxmltree::TextPos::new(1, 1)))
}

// A bounded lexical preflight, not a second XML parser. It counts open tags
// before roxmltree allocates a tree, ignoring quoted attributes, comments,
// CDATA and processing instructions. roxmltree subsequently owns all syntax
// validity. Declarations are refused here, before any entity can expand.
fn check_depth(file: &str, source: &str) -> Result<(), Error> {
    let bytes = source.as_bytes();
    let (mut offset, mut depth) = (0, 0usize);
    while let Some(next) = source[offset..].find('<') {
        offset += next;
        let tail = &source[offset..];
        let fail = |message| {
            let prefix = &source[..offset];
            let row = prefix.bytes().filter(|b| *b == b'\n').count() as u32 + 1;
            let col = prefix.rsplit('\n').next().unwrap_or("").chars().count() as u32 + 1;
            error(file, roxmltree::TextPos::new(row, col), message)
        };
        let delimiter = if tail.starts_with("<!--") {
            Some("-->")
        } else if tail.starts_with("<![CDATA[") {
            Some("]]>")
        } else if tail.starts_with("<?") {
            Some("?>")
        } else {
            None
        };
        if let Some(end) = delimiter {
            let Some(length) = tail.find(end) else { break };
            offset += length + end.len();
            continue;
        }
        if tail.starts_with("<!") {
            return Err(fail("XML declarations such as DOCTYPE are not allowed"));
        }
        let closing = tail.starts_with("</");
        let mut end = offset + 1;
        let mut quote = None;
        while end < bytes.len() {
            match (quote, bytes[end]) {
                (Some(q), b) if q == b => quote = None,
                (None, b'\'' | b'"') => quote = Some(bytes[end]),
                (None, b'>') => break,
                _ => {}
            }
            end += 1;
        }
        if end == bytes.len() {
            break;
        }
        if closing {
            depth = depth.saturating_sub(1);
        } else {
            if depth == MAX_DEPTH {
                return Err(fail("nesting exceeds 32 elements"));
            }
            if bytes[end - 1] != b'/' {
                depth += 1;
            }
        }
        offset = end + 1;
    }
    Ok(())
}

fn read(
    file: &str,
    node: roxmltree::Node<'_, '_>,
    root: bool,
    cursor: &mut (usize, roxmltree::TextPos),
) -> Result<MarkupElement, Error> {
    // Depth-first element order is source order; never rescan a long line for
    // every sibling when retaining locations at the admitted node limit.
    let start = node.range().start;
    for ch in node.document().input_text()[cursor.0..start].chars() {
        if ch == '\n' {
            cursor.1.row += 1;
            cursor.1.col = 1;
        } else {
            cursor.1.col += 1;
        }
    }
    cursor.0 = start;
    let pos = cursor.1;
    let tag = node.tag_name();
    let fail = |message: String| error(file, pos, format!("<{}>: {message}", tag.name()));
    if tag.namespace().is_some() || node.namespaces().len() != 0 {
        return Err(fail("XML namespaces are not allowed".into()));
    }
    let kind = match tag.name() {
        "tmt-view" if root => Kind::View,
        "tmt-row" => Kind::Row,
        "tmt-col" => Kind::Col,
        "tmt-cell" => Kind::Cell,
        "tmt-text" => Kind::Text,
        "tmt-repeat" => Kind::Repeat,
        "tmt-modal" => Kind::Modal,
        "tmt-scroll" => Kind::Scroll,
        "tmt-key-help" => Kind::KeyHelp,
        "tmt-list" => Kind::List,
        "tmt-table" => Kind::Table,
        "tmt-picker" => Kind::Picker,
        "tmt-switch" => Kind::Switch,
        "tmt-case" => Kind::Case,
        "tmt-default" => Kind::Default,
        _ => return Err(fail("unknown element or nested tmt-view".into())),
    };
    let leaf = matches!(kind, Kind::Cell | Kind::Text);
    let mut attributes: BTreeMap<String, String> = BTreeMap::new();
    for attr in node.attributes() {
        let name = attr.name();
        let allowed = if kind == Kind::Repeat {
            ["each", "as"].contains(&name)
        } else if matches!(kind, Kind::Switch | Kind::Case | Kind::Default) {
            // A branch is not a box: it takes no style, identity or token.
            (kind == Kind::Switch && name == "of") || (kind == Kind::Case && name == "min")
        } else {
            ["id", "id-bind", "class", "token", "selected"].contains(&name)
                || (matches!(kind, Kind::Modal | Kind::Picker)
                    && ["title", "placement"].contains(&name))
                || (kind == Kind::KeyHelp
                    && ["bind", "heading-token", "heading-bold", "section-gap"].contains(&name))
                || (matches!(kind, Kind::List | Kind::Table)
                    && ["bind", "as", "empty"].contains(&name))
                || (kind == Kind::Text && name == "slot")
                || (root && name == "version")
                || (kind == Kind::Row && ["row-id", "row-bind"].contains(&name))
                || (leaf
                    && ["column", "bind", "from", "format", "token-bind", "wrap"].contains(&name))
                || (kind == Kind::Cell && ["priority", "min-cols"].contains(&name))
                || (matches!(kind, Kind::Row | Kind::Col | Kind::Cell | Kind::Text)
                    && name == "hide-below")
        };
        if !allowed || attr.namespace().is_some() {
            return Err(fail(format!(
                "attribute {name}={:?} is not allowed",
                attr.value()
            )));
        }
        attributes.insert(name.into(), attr.value().into());
    }
    for (a, b) in [("id", "id-bind"), ("row-id", "row-bind"), ("bind", "from")] {
        if node.has_attribute(a) && node.has_attribute(b) {
            return Err(fail(format!("use either {a} or {b}")));
        }
    }
    if kind == Kind::Repeat {
        for key in ["each", "as"] {
            if node.attribute(key).is_none_or(|v| v.trim().is_empty()) {
                return Err(fail(format!("repeat requires nonempty {key}")));
            }
        }
    }
    let hide_below = attributes.remove("hide-below");
    match kind {
        Kind::Switch => {
            if let Some(of) = attributes.get("of")
                && !["container", "terminal"].contains(&of.as_str())
            {
                return Err(fail(format!("of={of:?}: expected container or terminal")));
            }
        }
        Kind::Case => {
            let Some(min) = attributes.get("min") else {
                return Err(fail("requires min=<breakpoint>".into()));
            };
            breakpoint(min).map_err(|why| fail(format!("min={min:?}: {why}")))?;
        }
        _ => {}
    }
    if let Some(name) = &hide_below {
        breakpoint(name).map_err(|why| fail(format!("hide-below={name:?}: {why}")))?;
    }
    let style = style::admit(kind, &attributes).map_err(fail)?;
    let text: String = node
        .children()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect();
    if !text.trim().is_empty()
        && (!leaf || node.has_attribute("bind") || node.has_attribute("from"))
    {
        return Err(fail(
            "literal text requires an unbound text/cell element".into(),
        ));
    }
    let mut children = Vec::new();
    for child in node.children().filter(|n| n.is_element()) {
        if leaf {
            return Err(fail("text/cell cannot contain elements".into()));
        }
        children.push(read(file, child, false, cursor)?);
    }
    let location = Location {
        line: pos.row,
        column: pos.col,
    };
    check_branches(file, kind, location, &children)?;
    let element = MarkupElement {
        kind,
        style,
        attributes,
        text,
        children,
        location,
    };
    // `hide-below="md"` is the one-optional-element form of a switch: the
    // element is the only case and the default is empty.
    Ok(match hide_below {
        Some(min) => {
            let branch = |kind, attributes: BTreeMap<String, String>, children| MarkupElement {
                kind,
                style: style::admit(kind, &attributes).expect("branches take no style"),
                attributes,
                text: String::new(),
                children,
                location,
            };
            branch(
                Kind::Switch,
                BTreeMap::new(),
                vec![
                    branch(
                        Kind::Case,
                        BTreeMap::from([("min".to_owned(), min)]),
                        vec![element],
                    ),
                    branch(Kind::Default, BTreeMap::new(), Vec::new()),
                ],
            )
        }
        None => element,
    })
}

fn breakpoint(name: &str) -> Result<tmt_cli_style::breakpoint::Breakpoint, String> {
    tmt_cli_style::breakpoint::by_name(name).ok_or_else(|| {
        let names = tmt_cli_style::breakpoint::ALL.map(|b| b.name);
        format!("expected a breakpoint name ({})", names.join(", "))
    })
}

/// A switch always renders exactly one branch: cases descend by `min` and a
/// trailing default covers everything narrower, so there are no gaps or overlaps.
fn check_branches(
    file: &str,
    kind: Kind,
    location: Location,
    children: &[MarkupElement],
) -> Result<(), Error> {
    let at = |location, message: String| Error {
        file: file.into(),
        location,
        message,
    };
    if kind != Kind::Switch {
        return match children
            .iter()
            .find(|child| matches!(child.kind, Kind::Case | Kind::Default))
        {
            Some(stray) => Err(at(
                stray.location,
                "branches belong directly inside <tmt-switch>".into(),
            )),
            None => Ok(()),
        };
    }
    let mut previous: Option<tmt_cli_style::breakpoint::Breakpoint> = None;
    let mut default = false;
    for (index, child) in children.iter().enumerate() {
        match child.kind {
            Kind::Case if !default => {
                let min = breakpoint(&child.attributes["min"]).expect("admitted min");
                if let Some(previous) = previous.filter(|previous| min.cells >= previous.cells) {
                    return Err(at(
                        child.location,
                        format!(
                            "<tmt-case min={:?}> must come before {:?}; cases descend and never repeat",
                            min.name, previous.name
                        ),
                    ));
                }
                previous = Some(min);
            }
            Kind::Default if index + 1 == children.len() && !default => default = true,
            Kind::Case | Kind::Default => {
                return Err(at(
                    child.location,
                    "<tmt-default> is the single last branch of a <tmt-switch>".into(),
                ));
            }
            _ => {
                return Err(at(
                    child.location,
                    "<tmt-switch> holds only <tmt-case> and <tmt-default>".into(),
                ));
            }
        }
    }
    if previous.is_none() {
        return Err(at(
            location,
            "<tmt-switch> needs at least one <tmt-case min=...>".into(),
        ));
    }
    if !default {
        return Err(at(
            location,
            "<tmt-switch> requires a final <tmt-default>".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
