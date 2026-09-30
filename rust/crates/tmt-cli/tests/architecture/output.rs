//! The #436 output guard: the CLI crates reach the terminal only through the
//! style layer (`tmt_cli_style::stream`), never through print macros, raw
//! standard handles or hand-written escape sequences. `docs/cli-style.md`
//! owns the rule; this is its syntactic check.

use super::source::{Source, production, production_impl, production_trait};
use proc_macro2::{TokenStream, TokenTree};
use std::collections::{BTreeMap, BTreeSet};
use syn::visit::{self, Visit};

/// Crates whose human output must go through the style layer.
/// `tmt-cli-style` renders, and `tmt-command-output` is core's output owner
/// that delegates to it, so both are outside this scope.
pub const GUARDED: &[&str] = &["tmt-cli", "tmt-office-command", "tmt-squad", "tmt-remote"];

/// A function whose output is an exact byte stream, not styled text. Only
/// that function is exempt; the rest of its file is still checked.
pub struct Exact {
    pub package: &'static str,
    pub file: &'static str,
    pub function: &'static str,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub package: String,
    pub file: String,
    /// The innermost named function, or `<module>` outside any.
    pub function: String,
    pub what: String,
}

pub fn findings(sources: &[Source]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for source in sources
        .iter()
        .filter(|source| GUARDED.contains(&source.package.as_str()))
    {
        let mut visitor = Visitor {
            source,
            functions: Vec::new(),
            findings: &mut findings,
        };
        visitor.visit_file(&source.syntax);
    }
    findings.sort();
    findings.dedup();
    findings
}

/// Violations of the guard given its two lists: an unexempt finding in an
/// unlisted file, a listed file with nothing left to migrate, or an exemption
/// that no longer matches a finding. Both lists only shrink.
pub fn violations(sources: &[Source], exact: &[Exact], migrating: &[(&str, &str)]) -> Vec<String> {
    let mut remaining: BTreeMap<(String, String), Vec<&Finding>> = BTreeMap::new();
    let found = findings(sources);
    let mut used = BTreeSet::new();
    for finding in &found {
        match exact.iter().position(|exact| {
            exact.package == finding.package
                && exact.file == finding.file
                && exact.function == finding.function
        }) {
            Some(index) => {
                used.insert(index);
            }
            None => remaining
                .entry((finding.package.clone(), finding.file.clone()))
                .or_default()
                .push(finding),
        }
    }
    let listed: BTreeSet<(String, String)> = migrating
        .iter()
        .map(|(package, file)| ((*package).to_owned(), (*file).to_owned()))
        .collect();
    let mut violations = Vec::new();
    for ((package, file), findings) in &remaining {
        if !listed.contains(&(package.clone(), file.clone())) {
            for finding in findings {
                violations.push(format!(
                    "{package}/{file}: {} in {} bypasses the style layer",
                    finding.what, finding.function
                ));
            }
        }
    }
    for (package, file) in &listed {
        if !remaining.contains_key(&(package.clone(), file.clone())) {
            violations.push(format!(
                "{package}/{file}: output goes through the style layer; remove it from the migration list"
            ));
        }
    }
    for (index, exact) in exact.iter().enumerate() {
        if exact.reason.trim().is_empty() {
            violations.push(format!(
                "{}/{}: exact-body exemption for {} needs a reason",
                exact.package, exact.file, exact.function
            ));
        }
        if !used.contains(&index) {
            violations.push(format!(
                "{}/{}: exact-body exemption for {} matches no output",
                exact.package, exact.file, exact.function
            ));
        }
    }
    violations
}

struct Visitor<'a> {
    source: &'a Source,
    functions: Vec<String>,
    findings: &'a mut Vec<Finding>,
}

impl Visitor<'_> {
    fn report(&mut self, what: String) {
        self.findings.push(Finding {
            package: self.source.package.clone(),
            file: self.source.file.clone(),
            function: self
                .functions
                .last()
                .cloned()
                .unwrap_or_else(|| "<module>".into()),
            what,
        });
    }

    fn within<T>(&mut self, name: String, walk: impl FnOnce(&mut Self) -> T) -> T {
        self.functions.push(name);
        let result = walk(self);
        self.functions.pop();
        result
    }

    /// Macro arguments are not parsed as Rust, so their tokens are scanned for
    /// the same handles and escapes.
    fn scan(&mut self, tokens: TokenStream) {
        let tokens: Vec<TokenTree> = tokens.into_iter().collect();
        for (index, token) in tokens.iter().enumerate() {
            match token {
                TokenTree::Group(group) => self.scan(group.stream()),
                TokenTree::Literal(literal) => {
                    if escape(&syn::Lit::new(literal.clone())) {
                        self.report("an escape sequence literal".into());
                    }
                }
                TokenTree::Ident(ident)
                    if matches!(ident.to_string().as_str(), "stdout" | "stderr") =>
                {
                    let io = index >= 3
                        && matches!(&tokens[index - 3], TokenTree::Ident(prefix) if prefix == "io")
                        && matches!(&tokens[index - 2], TokenTree::Punct(p) if p.as_char() == ':')
                        && matches!(&tokens[index - 1], TokenTree::Punct(p) if p.as_char() == ':');
                    if io {
                        self.report(format!("io::{ident}"));
                    }
                }
                _ => {}
            }
        }
    }
}

fn escape(literal: &syn::Lit) -> bool {
    match literal {
        syn::Lit::Str(value) => value.value().contains('\u{1b}'),
        syn::Lit::ByteStr(value) => value.value().contains(&0x1b),
        syn::Lit::CStr(value) => value.value().as_bytes().contains(&0x1b),
        syn::Lit::Char(value) => value.value() == '\u{1b}',
        syn::Lit::Byte(value) => value.value() == 0x1b,
        _ => false,
    }
}

fn standard_stream(segments: &[String]) -> Option<&str> {
    match segments {
        [.., io, stream] if io == "io" && matches!(stream.as_str(), "stdout" | "stderr") => {
            Some(stream)
        }
        _ => None,
    }
}

impl<'ast> Visit<'ast> for Visitor<'_> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if production(item) {
            visit::visit_item(self, item);
        }
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if production_impl(item) {
            visit::visit_impl_item(self, item);
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if production_trait(item) {
            visit::visit_trait_item(self, item);
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.within(item.sig.ident.to_string(), |this| {
            visit::visit_item_fn(this, item)
        });
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.within(item.sig.ident.to_string(), |this| {
            visit::visit_impl_item_fn(this, item)
        });
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.within(item.sig.ident.to_string(), |this| {
            visit::visit_trait_item_fn(this, item)
        });
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        if let Some(name) = node.path.segments.last().map(|s| s.ident.to_string())
            && matches!(name.as_str(), "print" | "println" | "eprint" | "eprintln")
        {
            self.report(format!("{name}!"));
        }
        self.scan(node.tokens.clone());
        visit::visit_macro(self, node);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        if let Some(stream) = standard_stream(&segments) {
            self.report(format!("io::{stream}"));
        }
        visit::visit_path(self, path);
    }

    fn visit_use_path(&mut self, node: &'ast syn::UsePath) {
        if node.ident == "io" {
            let mut names = Vec::new();
            collect_names(&node.tree, &mut names);
            for name in names {
                if matches!(name.as_str(), "stdout" | "stderr") {
                    self.report(format!("use io::{name}"));
                }
            }
        }
        visit::visit_use_path(self, node);
    }

    fn visit_lit(&mut self, literal: &'ast syn::Lit) {
        if escape(literal) {
            self.report("an escape sequence literal".into());
        }
        visit::visit_lit(self, literal);
    }
}

/// The names a `use io::…` tree imports directly (`io::{stdout, Write}`).
fn collect_names(tree: &syn::UseTree, names: &mut Vec<String>) {
    match tree {
        syn::UseTree::Name(name) => names.push(name.ident.to_string()),
        syn::UseTree::Rename(rename) => names.push(rename.ident.to_string()),
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                collect_names(tree, names);
            }
        }
        syn::UseTree::Path(_) | syn::UseTree::Glob(_) => {}
    }
}
