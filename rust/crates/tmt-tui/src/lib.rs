//! Internal TMT markup admission. Templates have checked structure and static
//! styles and binding are checked separately; geometry and painting remain later stages.

use std::{collections::BTreeMap, fmt};

pub mod binding;
pub mod geometry;
pub mod style;

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
        _ => return Err(fail("unknown element or nested tmt-view".into())),
    };
    let leaf = matches!(kind, Kind::Cell | Kind::Text);
    let mut attributes = BTreeMap::new();
    for attr in node.attributes() {
        let name = attr.name();
        let allowed = if kind == Kind::Repeat {
            ["each", "as"].contains(&name)
        } else {
            ["id", "id-bind", "class", "token", "selected"].contains(&name)
                || (root && name == "version")
                || (kind == Kind::Row && ["row-id", "row-bind"].contains(&name))
                || (leaf
                    && ["column", "bind", "from", "format", "token-bind", "wrap"].contains(&name))
                || (kind == Kind::Cell && ["priority", "min-cols"].contains(&name))
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
    Ok(MarkupElement {
        kind,
        style,
        attributes,
        text,
        children,
        location: Location {
            line: pos.row,
            column: pos.col,
        },
    })
}

#[cfg(test)]
mod tests;
