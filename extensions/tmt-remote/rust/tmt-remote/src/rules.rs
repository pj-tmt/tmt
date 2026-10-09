//! Firestore Rules and index composition for a `sharing` deploy plan (contract: Backends
//! and deploy, "Firestore Auth and Rules"). Pure: no I/O, clock or storage.
//!
//! An extension's admission artifact is a Rules *fragment*. Remote never splices its
//! bytes: it tokenizes the fragment, accepts only an allow-list of statements, identifiers,
//! operators and methods, and rebuilds the text from tokens inside a wrapper it owns
//! (`match /x/<extension>`), so the wrapper, `rules_version` and the default deny are
//! constants here. Anything not on the list is a refusal, because Rules offer ways around
//! a literal denylist: absolute `get`/`exists` paths, a first path segment built from a
//! string, `path()`, and user functions that shadow built-ins (emulator-verified, #2163).
//! Reads of the extension's own documents go through the `ext.get/exists/getAfter/
//! existsAfter` macros, which expand to a fixed `x/<extension>` prefix; an interpolated tail
//! stays one path segment.
//!
//! The boundary is confinement to the extension's root, not the quality of its logic: a
//! condition must still refer to `request.auth` or document data (a time-only or constant
//! condition is refused like `if true`), but what it checks is the extension's own.
use crate::{
    deploy_plan::{Plan, PlanView},
    limits,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// The artifact bytes of one extension in the plan.
#[derive(Clone, Copy, Debug)]
pub struct Fragment<'a> {
    pub extension: &'a str,
    pub source: &'a [u8],
}
/// The composed deploy artifacts: `firestore.rules` text and `firestore.indexes.json` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Composed {
    pub rules: String,
    pub indexes: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RulesReason {
    /// A plan extension has no fragment, or a fragment names none.
    MissingFragment,
    ExtraFragment,
    /// The fragment bytes do not hash to the digest the plan verified.
    FragmentDigest,
    /// Non-ASCII text, a control character or a character Rules syntax here never needs.
    Character,
    Unterminated,
    TooManyTokens,
    TooDeep,
    /// Unexpected token: the text is not in the accepted grammar.
    Syntax,
    /// A statement other than `match` or `allow` (`function`, `let`, `service`, ...).
    Statement,
    Method,
    WildcardName,
    /// An identifier, field root or call that is not on the allow-list.
    Identifier,
    Call,
    Macro,
    /// `true` anywhere except as an operand of `==` or `!=`.
    BareTrue,
    /// The condition refers to neither `request.auth` nor document data.
    Unconditional,
}
impl RulesReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::MissingFragment => "missing-fragment",
            Self::ExtraFragment => "extra-fragment",
            Self::FragmentDigest => "fragment-digest",
            Self::Character => "character",
            Self::Unterminated => "unterminated",
            Self::TooManyTokens => "too-many-tokens",
            Self::TooDeep => "too-deep",
            Self::Syntax => "syntax",
            Self::Statement => "statement",
            Self::Method => "method",
            Self::WildcardName => "wildcard-name",
            Self::Identifier => "identifier",
            Self::Call => "call",
            Self::Macro => "macro",
            Self::BareTrue => "bare-true",
            Self::Unconditional => "unconditional",
        }
    }
}
/// A refusal with the extension and the byte offset in its fragment, when there is one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RulesError {
    pub extension: Option<String>,
    pub offset: Option<usize>,
    pub reason: RulesReason,
}

type Result<T> = std::result::Result<T, (usize, RulesReason)>;

/// Compose the Rules and indexes of `plan` from the fragments it verified.
pub fn compose(
    plan: &Plan,
    fragments: &[Fragment<'_>],
) -> std::result::Result<Composed, RulesError> {
    let view = plan.view();
    let fail = |extension: Option<&str>, offset, reason| RulesError {
        extension: extension.map(str::to_owned),
        offset,
        reason,
    };
    let mut supplied: BTreeMap<&str, &[u8]> = BTreeMap::new();
    for fragment in fragments {
        if supplied
            .insert(fragment.extension, fragment.source)
            .is_some()
            || !view.extensions.iter().any(|e| e.name == fragment.extension)
        {
            return Err(fail(
                Some(fragment.extension),
                None,
                RulesReason::ExtraFragment,
            ));
        }
    }
    let mut rules = String::from(
        "rules_version = '2';\nservice cloud.firestore {\n  match /databases/{database}/documents {\n",
    );
    for extension in &view.extensions {
        let name = extension.name.as_str();
        let Some(source) = supplied.get(name) else {
            return Err(fail(Some(name), None, RulesReason::MissingFragment));
        };
        if sha256_hex(source) != extension.admission.artifact_digest {
            return Err(fail(Some(name), None, RulesReason::FragmentDigest));
        }
        let body = body(name, source).map_err(|(at, reason)| fail(Some(name), Some(at), reason))?;
        rules.push_str(&format!("    match /x/{name} {{\n"));
        if body.is_empty() {
            rules.push_str("      allow read, write: if false;\n");
        }
        rules.push_str(&body);
        rules.push_str("    }\n");
    }
    rules.push_str(
        "    match /{document=**} {\n      allow read, write: if false;\n    }\n  }\n}\n",
    );
    Ok(Composed {
        rules,
        indexes: indexes(view),
    })
}

/// `firestore.indexes.json`: one field override per (collection ID, field). Overrides are
/// keyed by collection ID, so extensions that use the same ID share a union, which only adds
/// indexes. A `ttlField` becomes a TTL policy only when the plan says the target provisions it.
fn indexes(view: &PlanView) -> String {
    let mut fields: BTreeMap<(String, String), (BTreeSet<&str>, bool)> = BTreeMap::new();
    for extension in &view.extensions {
        for resource in &extension.resources {
            let collection = resource.path.rsplit('/').next().unwrap_or_default();
            let key = |field: &str| (collection.to_owned(), field.to_owned());
            for index in &resource.indexes {
                fields
                    .entry(key(&index.field))
                    .or_default()
                    .0
                    .insert(index.direction);
            }
            if let (Some(field), true) = (&resource.ttl_field, view.capabilities.physical_ttl) {
                let entry = fields.entry(key(field)).or_default();
                entry.1 = true;
                if entry.0.is_empty() {
                    // A TTL field alone keeps Firestore's default single-field indexes.
                    entry.0.extend(["asc", "desc"]);
                }
            }
        }
    }
    let overrides: Vec<_> = fields
        .into_iter()
        .map(|((collection, field), (directions, ttl))| {
            let order = |direction: &&str| match *direction {
                "asc" => "ASCENDING",
                _ => "DESCENDING",
            };
            let listed: Vec<_> = directions
                .iter()
                .map(|d| json!({"order": order(d), "queryScope": "COLLECTION"}))
                .collect();
            let mut value = json!({"collectionGroup": collection, "fieldPath": field});
            if ttl {
                value["ttl"] = json!(true);
            }
            value["indexes"] = json!(listed);
            value
        })
        .collect();
    serde_json::to_string(&json!({"indexes": [], "fieldOverrides": overrides}))
        .expect("indexes serialize")
}

fn body(extension: &str, source: &[u8]) -> Result<String> {
    if !source
        .iter()
        .all(|b| matches!(b, b'\n' | b'\r' | b'\t' | b' '..=b'~'))
    {
        return Err((0, RulesReason::Character));
    }
    let text = std::str::from_utf8(source).map_err(|_| (0, RulesReason::Character))?;
    let tokens = lex(text)?;
    let mut parser = Parser {
        tokens,
        at: 0,
        extension,
        scope: Vec::new(),
        out: String::new(),
    };
    parser.items(0)?;
    if parser.at != parser.tokens.len() {
        return Err(parser.error(RulesReason::Syntax));
    }
    Ok(parser.out)
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Number(String),
    Str(String),
    Punct(&'static str),
}
#[derive(Clone, Debug)]
struct FragmentToken {
    tok: Tok,
    start: usize,
    end: usize,
}
const PUNCT: [&str; 28] = [
    "&&", "||", "==", "!=", "<=", ">=", "**", "{", "}", "(", ")", "[", "]", ".", ",", ";", ":",
    "?", "!", "<", ">", "+", "-", "*", "/", "%", "=", "$",
];

fn lex(text: &str) -> Result<Vec<FragmentToken>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
        } else if text[i..].starts_with("//") {
            i = text[i..].find('\n').map_or(bytes.len(), |n| i + n);
        } else if text[i..].starts_with("/*") {
            let Some(n) = text[i + 2..].find("*/") else {
                return Err((i, RulesReason::Unterminated));
            };
            i += n + 4;
        } else {
            let start = i;
            let tok = if b.is_ascii_alphabetic() || b == b'_' {
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                Tok::Ident(text[start..i].to_owned())
            } else if b.is_ascii_digit() {
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if bytes.get(i) == Some(&b'.') && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                Tok::Number(text[start..i].to_owned())
            } else if b == b'\'' || b == b'"' {
                i += 1;
                loop {
                    match bytes.get(i) {
                        None | Some(b'\n') => return Err((start, RulesReason::Unterminated)),
                        Some(b'\\') => i += 2,
                        Some(&q) if q == b => break,
                        Some(_) => i += 1,
                    }
                }
                i += 1;
                Tok::Str(text[start..i].to_owned())
            } else if let Some(p) = PUNCT.iter().find(|p| text[i..].starts_with(**p)) {
                i += p.len();
                Tok::Punct(p)
            } else {
                return Err((i, RulesReason::Character));
            };
            tokens.push(FragmentToken { tok, start, end: i });
            if tokens.len() > limits::RULES_FRAGMENT_TOKENS {
                return Err((start, RulesReason::TooManyTokens));
            }
        }
    }
    Ok(tokens)
}

/// Names a wildcard variable may not take: they would shadow what the allow-list reads.
const RESERVED: [&str; 14] = [
    "request",
    "resource",
    "database",
    "ext",
    "path",
    "get",
    "exists",
    "getAfter",
    "existsAfter",
    "duration",
    "int",
    "string",
    "true",
    "false",
];
const METHODS: [&str; 7] = ["get", "list", "read", "create", "update", "delete", "write"];
const IS_TYPES: [&str; 10] = [
    "string",
    "int",
    "float",
    "bool",
    "bytes",
    "list",
    "map",
    "timestamp",
    "duration",
    "null",
];
const MACROS: [&str; 4] = ["get", "exists", "getAfter", "existsAfter"];

/// A parsed operand or operator chain: its rebuilt text and what it refers to.
struct Typed {
    text: String,
    /// Refers to `request.auth`, document data or a macro read.
    data: bool,
    is_true: bool,
    string_literal: bool,
}
impl Typed {
    fn plain(text: String) -> Self {
        Self {
            text,
            data: false,
            is_true: false,
            string_literal: false,
        }
    }
}
/// Binding strength: `||` 1, `&&` 2, equality 3, relational 4, additive 5, multiplicative 6.
fn precedence(op: &str) -> u8 {
    match op {
        "||" => 1,
        "&&" => 2,
        "==" | "!=" => 3,
        "<" | "<=" | ">" | ">=" | "in" | "is" => 4,
        "+" | "-" => 5,
        _ => 6,
    }
}

struct Parser<'a> {
    tokens: Vec<FragmentToken>,
    at: usize,
    extension: &'a str,
    scope: Vec<String>,
    out: String,
}
impl Parser<'_> {
    fn error(&self, reason: RulesReason) -> (usize, RulesReason) {
        let offset = self
            .tokens
            .get(self.at)
            .or_else(|| self.tokens.last())
            .map_or(0, |t| t.start);
        (offset, reason)
    }
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.at).map(|t| &t.tok)
    }
    fn punct(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::Punct(q)) if *q == p)
    }
    fn word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(i)) if i == w)
    }
    fn take_punct(&mut self, p: &str) -> bool {
        let found = self.punct(p);
        if found {
            self.at += 1;
        }
        found
    }
    fn expect(&mut self, p: &str) -> Result<()> {
        if self.take_punct(p) {
            Ok(())
        } else {
            Err(self.error(RulesReason::Syntax))
        }
    }
    fn ident(&mut self) -> Result<String> {
        match self.peek() {
            Some(Tok::Ident(name)) => {
                let name = name.clone();
                self.at += 1;
                Ok(name)
            }
            _ => Err(self.error(RulesReason::Syntax)),
        }
    }

    /// Statements until the closing brace of the enclosing `match` (or the end of input at depth 0).
    fn items(&mut self, depth: usize) -> Result<()> {
        loop {
            match self.peek() {
                None if depth == 0 => return Ok(()),
                None => return Err(self.error(RulesReason::Syntax)),
                Some(Tok::Punct("}")) if depth > 0 => return Ok(()),
                Some(Tok::Ident(word)) if word == "match" => self.matching(depth)?,
                Some(Tok::Ident(word)) if word == "allow" => self.allow(depth)?,
                Some(_) => return Err(self.error(RulesReason::Statement)),
            }
        }
    }
    fn indent(depth: usize) -> String {
        " ".repeat(6 + 2 * depth)
    }

    fn matching(&mut self, depth: usize) -> Result<()> {
        if depth >= limits::RULES_FRAGMENT_DEPTH {
            return Err(self.error(RulesReason::TooDeep));
        }
        self.at += 1;
        let mut path = String::new();
        let mut added = 0;
        loop {
            self.expect("/")?;
            path.push('/');
            if self.take_punct("{") {
                let name = self.ident()?;
                if RESERVED.contains(&name.as_str()) || self.scope.contains(&name) {
                    return Err(self.error(RulesReason::WildcardName));
                }
                path.push('{');
                path.push_str(&name);
                let recursive = self.take_punct("=");
                if recursive {
                    self.expect("**")?;
                    path.push_str("=**");
                }
                self.expect("}")?;
                path.push('}');
                self.scope.push(name);
                added += 1;
                if recursive {
                    break;
                }
            } else {
                path.push_str(&self.literal_segment()?);
            }
            if !self.punct("/") {
                break;
            }
        }
        self.expect("{")?;
        let pad = Self::indent(depth);
        self.out.push_str(&format!("{pad}match {path} {{\n"));
        self.items(depth + 1)?;
        self.expect("}")?;
        self.out.push_str(&format!("{pad}}}\n"));
        self.scope.truncate(self.scope.len() - added);
        Ok(())
    }
    /// A literal path segment: adjacent words, numbers and hyphens, with no dot.
    fn literal_segment(&mut self) -> Result<String> {
        let mut segment = String::new();
        let mut end = None;
        while let Some(token) = self.tokens.get(self.at) {
            if end.is_some_and(|e| e != token.start) {
                break;
            }
            match &token.tok {
                Tok::Ident(s) => segment.push_str(s),
                Tok::Number(s) if !s.contains('.') => segment.push_str(s),
                Tok::Punct("-") => segment.push('-'),
                _ => break,
            }
            end = Some(token.end);
            self.at += 1;
        }
        if segment.is_empty() {
            return Err(self.error(RulesReason::Syntax));
        }
        Ok(segment)
    }

    fn allow(&mut self, depth: usize) -> Result<()> {
        self.at += 1;
        let mut methods = Vec::new();
        loop {
            let method = self.ident()?;
            if !METHODS.contains(&method.as_str()) {
                return Err(self.error(RulesReason::Method));
            }
            methods.push(method);
            if !self.take_punct(",") {
                break;
            }
        }
        self.expect(":")?;
        if !self.word("if") {
            return Err(self.error(RulesReason::Syntax));
        }
        self.at += 1;
        let condition = self.expr(0)?;
        self.expect(";")?;
        if condition.is_true {
            return Err(self.error(RulesReason::BareTrue));
        }
        if !condition.data {
            return Err(self.error(RulesReason::Unconditional));
        }
        let pad = Self::indent(depth);
        self.out.push_str(&format!(
            "{pad}allow {}: if {};\n",
            methods.join(", "),
            condition.text
        ));
        Ok(())
    }

    /// An operator chain checked as written: precedence only decides which operator a
    /// `true` operand belongs to, and the text is emitted in source order.
    fn expr(&mut self, nesting: usize) -> Result<Typed> {
        if nesting >= limits::RULES_FRAGMENT_NESTING {
            return Err(self.error(RulesReason::TooDeep));
        }
        let mut operands = vec![self.unary(nesting)?];
        let mut operators: Vec<&'static str> = Vec::new();
        while let Some(op) = self.binary_operator() {
            self.at += 1;
            operators.push(op);
            if op == "is" {
                let kind = self.ident()?;
                match IS_TYPES.iter().find(|t| **t == kind) {
                    Some(kind) => operands.push(Typed::plain((*kind).to_owned())),
                    None => return Err(self.error(RulesReason::Identifier)),
                }
            } else {
                operands.push(self.unary(nesting)?);
            }
        }
        for (i, operand) in operands.iter().enumerate() {
            if !operand.is_true {
                continue;
            }
            let before = i.checked_sub(1).map(|j| operators[j]);
            let after = operators.get(i).copied();
            let equality = |op: &str| op == "==" || op == "!=";
            let bound = (before.is_some_and(equality) && after.is_none_or(|a| precedence(a) <= 3))
                || (after.is_some_and(equality) && before.is_none_or(|b| precedence(b) <= 2));
            if !bound {
                return Err(self.error(RulesReason::BareTrue));
            }
        }
        let data = operands.iter().any(|o| o.data);
        let single = operators.is_empty();
        let mut text = operands[0].text.clone();
        for (op, operand) in operators.iter().zip(&operands[1..]) {
            text.push_str(&format!(" {op} {}", operand.text));
        }
        let mut typed = Typed {
            text,
            data,
            is_true: single && operands[0].is_true,
            string_literal: single && operands[0].string_literal,
        };
        if self.take_punct("?") {
            let yes = self.expr(nesting + 1)?;
            self.expect(":")?;
            let no = self.expr(nesting + 1)?;
            if typed.is_true || yes.is_true || no.is_true {
                return Err(self.error(RulesReason::BareTrue));
            }
            let data = typed.data || yes.data || no.data;
            typed = Typed {
                data,
                ..Typed::plain(format!("{} ? {} : {}", typed.text, yes.text, no.text))
            };
        }
        Ok(typed)
    }
    fn binary_operator(&self) -> Option<&'static str> {
        const OPS: [&str; 14] = [
            "||", "&&", "==", "!=", "<=", ">=", "<", ">", "+", "-", "*", "/", "%", "in",
        ];
        match self.peek()? {
            Tok::Punct(p) => OPS.iter().find(|o| *o == p).copied(),
            Tok::Ident(w) if w == "in" => Some("in"),
            Tok::Ident(w) if w == "is" => Some("is"),
            _ => None,
        }
    }

    fn unary(&mut self, nesting: usize) -> Result<Typed> {
        for op in ["!", "-"] {
            if self.take_punct(op) {
                let operand = self.unary(nesting + 1)?;
                if operand.is_true {
                    return Err(self.error(RulesReason::BareTrue));
                }
                return Ok(Typed {
                    text: format!("{op}{}", operand.text),
                    is_true: false,
                    string_literal: false,
                    ..operand
                });
            }
        }
        let primary = self.primary(nesting)?;
        self.postfix(primary, nesting)
    }

    fn primary(&mut self, nesting: usize) -> Result<Typed> {
        match self.peek().cloned() {
            Some(Tok::Number(n)) => {
                self.at += 1;
                Ok(Typed::plain(n))
            }
            Some(Tok::Str(s)) => {
                self.at += 1;
                Ok(Typed {
                    string_literal: true,
                    ..Typed::plain(s)
                })
            }
            Some(Tok::Punct("(")) => {
                self.at += 1;
                let inner = self.expr(nesting + 1)?;
                self.expect(")")?;
                if inner.is_true {
                    return Err(self.error(RulesReason::BareTrue));
                }
                Ok(Typed {
                    text: format!("({})", inner.text),
                    string_literal: false,
                    ..inner
                })
            }
            Some(Tok::Punct("[")) => {
                self.at += 1;
                let mut items = Vec::new();
                let mut data = false;
                while !self.punct("]") {
                    let item = self.expr(nesting + 1)?;
                    if item.is_true {
                        return Err(self.error(RulesReason::BareTrue));
                    }
                    data |= item.data;
                    items.push(item.text);
                    if !self.take_punct(",") {
                        break;
                    }
                }
                self.expect("]")?;
                Ok(Typed {
                    data,
                    ..Typed::plain(format!("[{}]", items.join(", ")))
                })
            }
            Some(Tok::Punct("{")) => {
                self.at += 1;
                let mut items = Vec::new();
                while !self.punct("}") {
                    let Some(Tok::Str(key)) = self.peek().cloned() else {
                        return Err(self.error(RulesReason::Syntax));
                    };
                    self.at += 1;
                    self.expect(":")?;
                    let value = self.expr(nesting + 1)?;
                    if value.is_true {
                        return Err(self.error(RulesReason::BareTrue));
                    }
                    items.push(format!("{key}: {}", value.text));
                    if !self.take_punct(",") {
                        break;
                    }
                }
                self.expect("}")?;
                Ok(Typed::plain(format!("{{{}}}", items.join(", "))))
            }
            Some(Tok::Ident(name)) => {
                self.at += 1;
                self.root(&name, nesting)
            }
            _ => Err(self.error(RulesReason::Syntax)),
        }
    }

    /// A value rooted at an identifier on the allow-list.
    fn root(&mut self, name: &str, nesting: usize) -> Result<Typed> {
        match name {
            "null" | "false" => Ok(Typed::plain(name.to_owned())),
            "true" => Ok(Typed {
                is_true: true,
                ..Typed::plain("true".into())
            }),
            "request" => {
                self.expect(".")?;
                let field = self.ident()?;
                match field.as_str() {
                    "auth" => Ok(Typed {
                        data: true,
                        ..Typed::plain("request.auth".into())
                    }),
                    "time" | "method" => Ok(Typed::plain(format!("request.{field}"))),
                    "resource" => {
                        self.expect(".")?;
                        if self.ident()? != "data" {
                            return Err(self.error(RulesReason::Identifier));
                        }
                        Ok(Typed {
                            data: true,
                            ..Typed::plain("request.resource.data".into())
                        })
                    }
                    _ => Err(self.error(RulesReason::Identifier)),
                }
            }
            "resource" => {
                self.expect(".")?;
                let field = self.ident()?;
                if field != "data" && field != "id" {
                    return Err(self.error(RulesReason::Identifier));
                }
                Ok(Typed {
                    data: true,
                    ..Typed::plain(format!("resource.{field}"))
                })
            }
            "duration" => {
                self.expect(".")?;
                if self.ident()? != "value" {
                    return Err(self.error(RulesReason::Call));
                }
                let args = self.arguments(nesting)?;
                if args.len() != 2 {
                    return Err(self.error(RulesReason::Call));
                }
                Ok(Typed::plain(format!("duration.value({})", join(&args))))
            }
            "int" | "string" => {
                let args = self.arguments(nesting)?;
                if args.len() != 1 {
                    return Err(self.error(RulesReason::Call));
                }
                let data = args[0].data;
                Ok(Typed {
                    data,
                    ..Typed::plain(format!("{name}({})", join(&args)))
                })
            }
            "ext" => self.macro_call(nesting),
            other if self.scope.iter().any(|v| v == other) => Ok(Typed::plain(other.to_owned())),
            _ => Err(self.error(RulesReason::Identifier)),
        }
    }

    /// `ext.get(/a/$(x))`: reads under the fixed `x/<extension>` prefix only.
    fn macro_call(&mut self, nesting: usize) -> Result<Typed> {
        self.expect(".")?;
        let name = self.ident()?;
        if !MACROS.contains(&name.as_str()) {
            return Err(self.error(RulesReason::Macro));
        }
        self.expect("(")?;
        let mut path = format!("/databases/$(database)/documents/x/{}", self.extension);
        let mut segments = 0;
        while self.punct("/") {
            self.at += 1;
            path.push('/');
            if self.take_punct("$") {
                self.expect("(")?;
                let inner = self.expr(nesting + 1)?;
                self.expect(")")?;
                if inner.is_true {
                    return Err(self.error(RulesReason::BareTrue));
                }
                path.push_str(&format!("$({})", inner.text));
            } else {
                path.push_str(
                    &self
                        .literal_segment()
                        .map_err(|(at, _)| (at, RulesReason::Macro))?,
                );
            }
            segments += 1;
        }
        self.expect(")")?;
        if segments == 0 {
            return Err(self.error(RulesReason::Macro));
        }
        Ok(Typed {
            data: true,
            ..Typed::plain(format!("{name}({path})"))
        })
    }

    fn arguments(&mut self, nesting: usize) -> Result<Vec<Typed>> {
        self.expect("(")?;
        let mut args = Vec::new();
        while !self.punct(")") {
            let arg = self.expr(nesting + 1)?;
            if arg.is_true {
                return Err(self.error(RulesReason::BareTrue));
            }
            args.push(arg);
            if !self.take_punct(",") {
                break;
            }
        }
        self.expect(")")?;
        Ok(args)
    }

    /// `.field`, `[index]` and the allow-listed methods. A bare `get(` never reaches here:
    /// `get` is a method only after a `.`, and the `ext` receiver is handled by `macro_call`.
    fn postfix(&mut self, mut value: Typed, nesting: usize) -> Result<Typed> {
        loop {
            if self.take_punct(".") {
                let name = self.ident()?;
                if self.punct("(") {
                    let (min, max, literal) = match name.as_str() {
                        "size" | "keys" | "values" | "affectedKeys" | "toMillis" => (0, 0, false),
                        "hasAll" | "hasAny" | "hasOnly" | "diff" => (1, 1, false),
                        "matches" => (1, 1, true),
                        "get" => (1, 2, false),
                        _ => return Err(self.error(RulesReason::Method)),
                    };
                    let args = self.arguments(nesting)?;
                    if args.len() < min || args.len() > max || (literal && !args[0].string_literal)
                    {
                        return Err(self.error(RulesReason::Call));
                    }
                    value = Typed {
                        text: format!("{}.{name}({})", value.text, join(&args)),
                        data: value.data,
                        is_true: false,
                        string_literal: false,
                    };
                } else {
                    value = Typed {
                        text: format!("{}.{name}", value.text),
                        is_true: false,
                        string_literal: false,
                        ..value
                    };
                }
            } else if self.take_punct("[") {
                let index = self.expr(nesting + 1)?;
                self.expect("]")?;
                value = Typed {
                    text: format!("{}[{}]", value.text, index.text),
                    is_true: false,
                    string_literal: false,
                    ..value
                };
            } else {
                return Ok(value);
            }
            if value.text.len() > limits::DECLARATION_ARTIFACT_BYTES {
                return Err(self.error(RulesReason::TooManyTokens));
            }
        }
    }
}
fn join(args: &[Typed]) -> String {
    args.iter()
        .map(|a| a.text.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}
