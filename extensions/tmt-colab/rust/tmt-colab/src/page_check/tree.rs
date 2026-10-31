//! The browser's parse of a page source as a small arena tree.
//!
//! `html5ever` is Servo's implementation of the HTML tree-construction algorithm, the one the
//! browser runs for `template.innerHTML`. Using it keeps every verdict of the checker on the
//! browser's tree, not on a guess about it: formatting-element reconstruction, implied end tags,
//! foster parenting and foreign content all land where the browser puts them.
//!
//! The arena is the only `TreeSink`: nodes are indices, parents are indices, and nothing is
//! recursive, so a deeply nested source can neither overflow the stack nor on drop.

use std::{borrow::Cow, cell::Cell, cell::RefCell, collections::HashSet, rc::Rc};

use crate::limits::PAGE_DEPTH;

use html5ever::{
    Attribute, ExpandedName, ParseOpts, QualName, TokenizerResult,
    buffer_queue::BufferQueue,
    local_name, ns, parse_fragment,
    tendril::{ByteTendril, StrTendril, TendrilSink},
    tokenizer::{TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts},
    tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeBuilder, TreeBuilderOpts, TreeSink},
};

fn fragment_options() -> ParseOpts {
    ParseOpts {
        tree_builder: TreeBuilderOpts {
            scripting_enabled: false,
            ..TreeBuilderOpts::default()
        },
        ..ParseOpts::default()
    }
}

/// A tokenizer-emitted start tag, with an exact original byte range when consumption boundaries
/// identify it. No text search repairs a missing range; callers leave that source unchanged.
pub(crate) struct SourceTag {
    pub name: String,
    pub range: Option<std::ops::Range<usize>>,
}

/// The checker's fragment parser, with a transparent token observer for source-preserving copies.
/// Ordinary checks keep their existing fast parse; only a template copy needs source positions.
pub(crate) fn start_tags(source: &str) -> Option<Vec<SourceTag>> {
    let deep = Rc::new(Cell::new(None));
    let sink = Sink::new(Rc::clone(&deep));
    let parser = parse_fragment(
        sink,
        fragment_options(),
        QualName::new(None, ns!(html), local_name!("template")),
        Vec::new(),
        false,
    );
    let builder = parser.tokenizer.sink;
    let options = TokenizerOpts {
        initial_state: Some(builder.tokenizer_state_for_context_elem(false)),
        ..TokenizerOpts::default()
    };
    let input = Rc::new(BufferQueue::default());
    let fed = Rc::new(Cell::new(0));
    let tokenizer = Tokenizer::new(
        SourceTokens {
            builder,
            source,
            input: Rc::clone(&input),
            fed: Rc::clone(&fed),
            boundary: Cell::new(0),
            tags: RefCell::new(Vec::new()),
        },
        options,
    );
    let mut at = 0;
    while at < source.len() {
        let mut end = (at + 16 * 1024).min(source.len());
        while !source.is_char_boundary(end) {
            end -= 1;
        }
        fed.set(end);
        input.push_back(StrTendril::from_slice(&source[at..end]));
        while !matches!(tokenizer.feed(&input), TokenizerResult::Done) {}
        if deep.get().is_some() {
            return None;
        }
        at = end;
    }
    tokenizer.end();
    Some(tokenizer.sink.tags.into_inner())
}

struct SourceTokens<'a> {
    builder: TreeBuilder<Handle, Sink>,
    source: &'a str,
    input: Rc<BufferQueue>,
    fed: Rc<Cell<usize>>,
    boundary: Cell<usize>,
    tags: RefCell<Vec<SourceTag>>,
}
impl TokenSink for SourceTokens<'_> {
    type Handle = Handle;

    fn process_token(&self, token: Token, line: u64) -> TokenSinkResult<Handle> {
        if !matches!(token, Token::ParseError(_)) {
            // BufferQueue clones share tendril bytes, not consumption. Include pending lookahead
            // buffers: offsets are bytes of the original input, never decoded token text lengths.
            let remaining = self.input.as_ref().clone();
            let bytes: usize = std::iter::from_fn(|| remaining.pop_front())
                .map(|chunk| chunk.len())
                .sum();
            let end = self.fed.get() - bytes;
            let mut start = self.boundary.replace(end);
            // The tokenizer discards the initial BOM and the LF of CRLF without emitting tokens.
            if start == 0 && self.source.starts_with('\u{feff}') {
                start += '\u{feff}'.len_utf8();
            }
            if start > 0
                && self.source.as_bytes().get(start - 1) == Some(&b'\r')
                && self.source.as_bytes().get(start) == Some(&b'\n')
            {
                start += 1;
            }
            if let Token::TagToken(tag) = &token
                && tag.kind == TagKind::StartTag
            {
                let range = self.source.get(start..end).and_then(|raw| {
                    let name = raw.get(1..1 + tag.name.len())?;
                    (raw.starts_with('<')
                        && name.eq_ignore_ascii_case(&tag.name)
                        && raw
                            .as_bytes()
                            .get(1 + tag.name.len())
                            .is_some_and(|c| c.is_ascii_whitespace() || matches!(c, b'/' | b'>'))
                        && raw.ends_with('>'))
                    .then_some(start..end)
                });
                self.tags.borrow_mut().push(SourceTag {
                    name: tag.name.to_string(),
                    range,
                });
            }
        }
        self.builder.process_token(token, line)
    }

    fn end(&self) {
        self.builder.end();
    }

    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        self.builder
            .adjusted_current_node_present_but_not_in_html_namespace()
    }
}

pub(super) type Id = usize;

struct ParsedElement {
    name: String,
    html: bool,
    attrs: Vec<(String, String)>,
    /// The line of the end of the start tag, as the tokenizer counted it.
    line: u64,
    /// The separate document fragment holding a `<template>`'s content.
    template: Option<Id>,
    mathml_annotation_xml_integration_point: bool,
}

enum Kind {
    Document,
    /// The content of a `<template>`: reachable only through its element, never as a child.
    Fragment,
    Element(ParsedElement),
    Text(String),
    /// Comments, processing instructions and doctypes: nothing a rule reads.
    Other,
}

struct Node {
    kind: Kind,
    parent: Option<Id>,
    children: Vec<Id>,
    /// Elements from the top of the source down to this node, set when it is attached.
    depth: usize,
}

/// A parsed page source. Elements are the ones a `querySelectorAll` on the template content would
/// reach: nested template contents are not entered.
pub(super) struct Tree {
    nodes: Vec<Node>,
    /// The element the fragment parser puts the parsed nodes under; it is not part of the source.
    root: Id,
}

impl Tree {
    /// Parse `source` as the browser parses `template.innerHTML`: a fragment in a `template`
    /// context, with scripting off (so `<noscript>` content is markup).
    ///
    /// Nesting past [`PAGE_DEPTH`] stops the parse: the tree builder scans its stack of open
    /// elements on every start tag, so a source of a few hundred thousand `<div>` would take
    /// minutes, and a browser stops nesting at the same depth, so no verdict past it would
    /// match anyway. The source is fed in chunks so the depth is noticed within one chunk.
    pub(super) fn parse(source: &str) -> Result<Tree, TooDeep> {
        const CHUNK: usize = 16 * 1024;
        let options = fragment_options();
        let context = QualName::new(None, ns!(html), local_name!("template"));
        let deep = Rc::new(Cell::new(None));
        let sink = Sink::new(Rc::clone(&deep));
        let nodes = Rc::clone(&sink.nodes);
        let mut parser = parse_fragment(sink, options, context, Vec::new(), false).from_utf8();
        for chunk in source.as_bytes().chunks(CHUNK) {
            parser.process(ByteTendril::from_slice(chunk));
            if let Some(too_deep) = deep.get() {
                return Err(too_deep.at(&nodes.borrow()));
            }
        }
        Ok(parser.finish())
    }

    /// Every element in document order.
    pub(super) fn elements(&self) -> Vec<Id> {
        let mut found = Vec::new();
        let mut stack: Vec<Id> = self.nodes[self.root]
            .children
            .iter()
            .rev()
            .copied()
            .collect();
        while let Some(id) = stack.pop() {
            if matches!(self.nodes[id].kind, Kind::Element(_)) {
                found.push(id);
            }
            stack.extend(self.nodes[id].children.iter().rev());
        }
        found
    }

    fn element(&self, id: Id) -> &ParsedElement {
        match &self.nodes[id].kind {
            Kind::Element(element) => element,
            _ => unreachable!("rules only hold element ids"),
        }
    }

    pub(super) fn name(&self, id: Id) -> &str {
        &self.element(id).name
    }

    pub(super) fn html_element(&self, id: Id) -> bool {
        self.element(id).html
    }

    pub(super) fn line(&self, id: Id) -> u64 {
        self.element(id).line
    }

    pub(super) fn attr(&self, id: Id, name: &str) -> Option<&str> {
        self.element(id)
            .attrs
            .iter()
            .find(|(attr, _)| attr == name)
            .map(|(_, value)| value.as_str())
    }

    pub(super) fn attributes(&self, id: Id) -> impl Iterator<Item = (&str, &str)> {
        self.element(id)
            .attrs
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    pub(super) fn child_elements(&self, id: Id) -> impl Iterator<Item = Id> + '_ {
        self.nodes[id]
            .children
            .iter()
            .copied()
            .filter(|child| matches!(self.nodes[*child].kind, Kind::Element(_)))
    }

    /// The parent element, or `None` at the top of the source (the browser's `parentElement` of
    /// a node directly in the template content).
    pub(super) fn parent_element(&self, id: Id) -> Option<Id> {
        self.nodes[id].parent.filter(|parent| {
            *parent != self.root && matches!(self.nodes[*parent].kind, Kind::Element(_))
        })
    }

    /// The ancestor elements, nearest first.
    pub(super) fn ancestors(&self, id: Id) -> impl Iterator<Item = Id> + '_ {
        std::iter::successors(self.parent_element(id), |ancestor| {
            self.parent_element(*ancestor)
        })
    }

    pub(super) fn has_ancestor(&self, id: Id, name: &str) -> bool {
        self.ancestors(id)
            .any(|ancestor| self.name(ancestor) == name)
    }

    /// The nearest ancestor element with one of these names, as `closest` finds it from the parent.
    pub(super) fn closest(&self, id: Id, names: &[&str]) -> Option<Id> {
        self.ancestors(id)
            .find(|ancestor| names.contains(&self.name(*ancestor)))
    }

    /// Text nodes in document order, paired with their parent element. Template contents are
    /// separate fragments and never appear in the renderer's body TreeWalker.
    pub(super) fn text_nodes(&self) -> Vec<(Option<Id>, &str)> {
        let mut found = Vec::new();
        let mut stack: Vec<_> = self.nodes[self.root]
            .children
            .iter()
            .rev()
            .copied()
            .collect();
        while let Some(id) = stack.pop() {
            match &self.nodes[id].kind {
                Kind::Text(text) => {
                    let parent = self.nodes[id]
                        .parent
                        .filter(|parent| matches!(self.nodes[*parent].kind, Kind::Element(_)));
                    found.push((parent, text.as_str()))
                }
                Kind::Element(_) => stack.extend(self.nodes[id].children.iter().rev()),
                _ => {}
            }
        }
        found
    }
    pub(super) fn excluded(&self, id: Id) -> bool {
        matches!(
            self.name(id),
            "head" | "script" | "style" | "template" | "noscript"
        ) || ["hidden", "data-colab-highlight", "data-colab-image"]
            .iter()
            .any(|name| self.attr(id, name).is_some())
    }

    /// Whether the element contains plain text only, without markup or comments.
    pub(super) fn plain_text(&self, id: Id) -> bool {
        self.nodes[id]
            .children
            .iter()
            .all(|child| matches!(self.nodes[*child].kind, Kind::Text(_)))
    }

    /// The `textContent` of an element: every descendant text node, comments excluded.
    pub(super) fn text(&self, id: Id) -> String {
        self.text_excluding_children(id, &[])
    }

    /// `textContent` with named immediate children (including their descendants) left out.
    pub(super) fn text_excluding_children(&self, id: Id, excluded: &[Id]) -> String {
        let excluded: HashSet<Id> = excluded.iter().copied().collect();
        let mut text = String::new();
        let mut stack: Vec<Id> = self.nodes[id]
            .children
            .iter()
            .rev()
            .copied()
            .filter(|child| !excluded.contains(child))
            .collect();
        while let Some(next) = stack.pop() {
            match &self.nodes[next].kind {
                Kind::Text(value) => text.push_str(value),
                Kind::Element(_) => stack.extend(self.nodes[next].children.iter().rev()),
                _ => {}
            }
        }
        text
    }
}

/// The element that nested past [`PAGE_DEPTH`], for the finding that says so.
pub(super) struct TooDeep {
    pub name: String,
    /// Its 1-based index among the attached elements of that name, in creation order.
    pub ordinal: usize,
    pub id: Option<String>,
    pub line: u64,
}

/// The first node found too deep, before its name is looked up.
#[derive(Clone, Copy)]
struct Overflow(Id);

impl Overflow {
    fn at(&self, nodes: &[Node]) -> TooDeep {
        let Kind::Element(element) = &nodes[self.0].kind else {
            unreachable!("only elements are counted for depth");
        };
        let ordinal = nodes[..=self.0]
            .iter()
            .filter(|node| {
                node.parent.is_some()
                    && matches!(&node.kind, Kind::Element(other) if other.name == element.name)
            })
            .count();
        TooDeep {
            name: element.name.clone(),
            ordinal,
            id: element
                .attrs
                .iter()
                .find(|(name, _)| name == "data-id")
                .map(|(_, value)| value.clone()),
            line: element.line,
        }
    }
}

#[derive(Clone)]
struct Handle {
    id: Id,
    /// `elem_name` hands out a borrow of the handle, so the handle carries the name.
    name: Rc<QualName>,
}

struct Sink {
    /// Shared so the caller still holds the nodes when it stops a parse early.
    nodes: Rc<RefCell<Vec<Node>>>,
    line: Cell<u64>,
    unnamed: Rc<QualName>,
    deep: Rc<Cell<Option<Overflow>>>,
}

impl Sink {
    fn new(deep: Rc<Cell<Option<Overflow>>>) -> Sink {
        Sink {
            nodes: Rc::new(RefCell::new(vec![node(Kind::Document)])),
            line: Cell::new(1),
            unnamed: Rc::new(QualName::new(None, ns!(), local_name!(""))),
            deep,
        }
    }

    /// Note the first element nested past the cap; the caller stops feeding the parser.
    fn watch_depth(&self, nodes: &[Node], child: Id) {
        // The fragment's `html` root sits at depth 1; the source starts below it.
        if nodes[child].depth > PAGE_DEPTH + 1
            && matches!(nodes[child].kind, Kind::Element(_))
            && self.deep.get().is_none()
        {
            self.deep.set(Some(Overflow(child)));
        }
    }

    fn handle(&self, id: Id) -> Handle {
        Handle {
            id,
            name: Rc::clone(&self.unnamed),
        }
    }
}

fn node(kind: Kind) -> Node {
    Node {
        kind,
        parent: None,
        children: Vec::new(),
        depth: 0,
    }
}

fn detach(nodes: &mut [Node], id: Id) {
    if let Some(parent) = nodes[id].parent.take() {
        nodes[parent].children.retain(|child| *child != id);
    }
}

fn link(nodes: &mut [Node], parent: Id, child: Id, at: Option<usize>) {
    nodes[child].parent = Some(parent);
    nodes[child].depth = nodes[parent].depth + 1;
    match at {
        Some(index) => nodes[parent].children.insert(index, child),
        None => nodes[parent].children.push(child),
    }
}

/// Append `text` to `previous` when that is a text node, as adjacent text must be one node.
fn extend_text(nodes: &mut [Node], previous: Option<Id>, text: &str) -> bool {
    if let Some(Kind::Text(existing)) = previous.map(|id| &mut nodes[id].kind) {
        existing.push_str(text);
        return true;
    }
    false
}

impl TreeSink for Sink {
    type Handle = Handle;
    type Output = Tree;
    type ElemName<'a> = ExpandedName<'a>;

    fn finish(self) -> Tree {
        let nodes = std::mem::take(&mut *self.nodes.borrow_mut());
        // The fragment parser puts everything under one `html` element child of the document.
        let root = nodes[0]
            .children
            .iter()
            .copied()
            .find(|id| matches!(nodes[*id].kind, Kind::Element(_)))
            .unwrap_or(0);
        Tree { nodes, root }
    }

    fn parse_error(&self, _message: Cow<'static, str>) {}

    fn get_document(&self) -> Handle {
        self.handle(0)
    }

    fn elem_name<'a>(&'a self, target: &'a Handle) -> ExpandedName<'a> {
        target.name.expanded()
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> Handle {
        let mut nodes = self.nodes.borrow_mut();
        let template = flags.template.then(|| {
            nodes.push(node(Kind::Fragment));
            nodes.len() - 1
        });
        nodes.push(node(Kind::Element(ParsedElement {
            name: name.local.to_string(),
            html: name.ns == ns!(html),
            attrs: attrs
                .into_iter()
                .map(|attr| (attr.name.local.to_string(), attr.value.to_string()))
                .collect(),
            line: self.line.get(),
            template,
            mathml_annotation_xml_integration_point: flags.mathml_annotation_xml_integration_point,
        })));
        Handle {
            id: nodes.len() - 1,
            name: Rc::new(name),
        }
    }

    fn create_comment(&self, _text: StrTendril) -> Handle {
        let mut nodes = self.nodes.borrow_mut();
        nodes.push(node(Kind::Other));
        self.handle(nodes.len() - 1)
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Handle {
        let mut nodes = self.nodes.borrow_mut();
        nodes.push(node(Kind::Other));
        self.handle(nodes.len() - 1)
    }

    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        let mut nodes = self.nodes.borrow_mut();
        match child {
            NodeOrText::AppendText(text) => {
                let last = nodes[parent.id].children.last().copied();
                if !extend_text(&mut nodes, last, &text) {
                    nodes.push(node(Kind::Text(text.to_string())));
                    let id = nodes.len() - 1;
                    link(&mut nodes, parent.id, id, None);
                }
            }
            NodeOrText::AppendNode(child) => {
                link(&mut nodes, parent.id, child.id, None);
                self.watch_depth(&nodes, child.id);
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev_element: &Handle,
        child: NodeOrText<Handle>,
    ) {
        let has_parent = self.nodes.borrow()[element.id].parent.is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(&self, _: StrTendril, _: StrTendril, _: StrTendril) {}

    fn get_template_contents(&self, target: &Handle) -> Handle {
        let nodes = self.nodes.borrow();
        match &nodes[target.id].kind {
            Kind::Element(ParsedElement {
                template: Some(contents),
                ..
            }) => self.handle(*contents),
            _ => unreachable!("the tree builder asks only a template for its contents"),
        }
    }

    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        x.id == y.id
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &Handle, new_node: NodeOrText<Handle>) {
        let mut nodes = self.nodes.borrow_mut();
        let Some(parent) = nodes[sibling.id].parent else {
            return;
        };
        let position = |nodes: &[Node]| {
            nodes[parent]
                .children
                .iter()
                .position(|id| *id == sibling.id)
        };
        match new_node {
            NodeOrText::AppendText(text) => {
                let index = position(&nodes).unwrap_or(0);
                let previous = index
                    .checked_sub(1)
                    .map(|before| nodes[parent].children[before]);
                if !extend_text(&mut nodes, previous, &text) {
                    nodes.push(node(Kind::Text(text.to_string())));
                    let id = nodes.len() - 1;
                    link(&mut nodes, parent, id, Some(index));
                }
            }
            NodeOrText::AppendNode(moved) => {
                // The moved node may be an earlier sibling, so the index is read after detaching.
                detach(&mut nodes, moved.id);
                let index = position(&nodes).unwrap_or(nodes[parent].children.len());
                link(&mut nodes, parent, moved.id, Some(index));
                self.watch_depth(&nodes, moved.id);
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        let mut nodes = self.nodes.borrow_mut();
        if let Kind::Element(element) = &mut nodes[target.id].kind {
            for attr in attrs {
                let name = attr.name.local.to_string();
                if !element.attrs.iter().any(|(existing, _)| *existing == name) {
                    element.attrs.push((name, attr.value.to_string()));
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &Handle) {
        detach(&mut self.nodes.borrow_mut(), target.id);
    }

    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        let mut nodes = self.nodes.borrow_mut();
        let moved = std::mem::take(&mut nodes[node.id].children);
        for child in moved {
            link(&mut nodes, new_parent.id, child, None);
        }
    }

    fn is_mathml_annotation_xml_integration_point(&self, handle: &Handle) -> bool {
        matches!(
            &self.nodes.borrow()[handle.id].kind,
            Kind::Element(ParsedElement {
                mathml_annotation_xml_integration_point: true,
                ..
            })
        )
    }

    fn set_current_line(&self, line: u64) {
        self.line.set(line);
    }
}

#[cfg(test)]
mod source_range_tests {
    use super::*;

    #[test]
    fn token_ranges_exclude_attribute_and_script_lookalikes() {
        let prefix = "<div title=\"<tmt-question data-agent='attribute'>\"></div><script>let s = \"<tmt-question data-agent='script'>\";</script>";
        let tag = "<TMT-QUESTION title=\"a > b\" data-agent='old'>";
        let source = format!("{prefix}{tag}");
        let tags = start_tags(&source).unwrap();
        let questions: Vec<_> = tags
            .iter()
            .filter(|tag| tag.name == "tmt-question")
            .collect();
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].range, Some(prefix.len()..source.len()));
    }

    #[test]
    fn a_reconsumed_less_than_sign_does_not_guess_a_start_range() {
        let tags = start_tags("<<tmt-question data-agent='old'>").unwrap();
        let question = tags.iter().find(|tag| tag.name == "tmt-question").unwrap();
        assert!(question.range.is_none());
    }

    #[test]
    fn chunked_parser_positions_are_original_utf8_byte_offsets() {
        let prefix = format!("<p>{}</p>\r\n", "é".repeat(8190));
        let tag = "<tmt-question data-agent='old'>";
        let source = format!("{prefix}{tag}");
        let tags = start_tags(&source).unwrap();
        let question = tags.iter().find(|tag| tag.name == "tmt-question").unwrap();
        assert_eq!(question.range, Some(prefix.len()..source.len()));
    }
}
