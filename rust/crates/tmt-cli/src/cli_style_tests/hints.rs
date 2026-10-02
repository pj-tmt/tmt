//! Source coverage for presentation-owned hint samples. Reads production literals,
//! never executes their contents or dispatches commands.
use proc_macro2::{TokenStream, TokenTree};
use std::{collections::BTreeSet, path::Path};
use syn::visit::Visit;

pub(crate) struct HintSpec {
    pub template: &'static str,
    pub ends: &'static [&'static str],
    pub substitutions: &'static [(&'static str, &'static str)],
    pub skip: Option<&'static str>,
}

impl HintSpec {
    pub const fn core(
        template: &'static str,
        ends: &'static [&'static str],
        substitutions: &'static [(&'static str, &'static str)],
    ) -> Self {
        Self {
            template,
            ends,
            substitutions,
            skip: None,
        }
    }
    pub const fn skipped(template: &'static str, reason: &'static str) -> Self {
        Self {
            template,
            ends: &[],
            substitutions: &[],
            skip: Some(reason),
        }
    }
}

fn test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .meta
                .require_list()
                .is_ok_and(|list| list.tokens.to_string() == "test")
    })
}

#[derive(Default)]
struct Literals(BTreeSet<String>);
impl Literals {
    fn tokens(&mut self, tokens: TokenStream) {
        for token in tokens {
            match token {
                TokenTree::Group(group) => self.tokens(group.stream()),
                TokenTree::Literal(literal) => {
                    if let Ok(literal) = syn::parse_str::<syn::LitStr>(&literal.to_string()) {
                        self.record(literal.value());
                    }
                }
                _ => {}
            }
        }
    }
    fn record(&mut self, text: String) {
        if text.contains("tmt ") {
            self.0.insert(text);
        }
    }
}
impl<'ast> Visit<'ast> for Literals {
    fn visit_attribute(&mut self, _: &'ast syn::Attribute) {}
    fn visit_item(&mut self, item: &'ast syn::Item) {
        let attributes = match item {
            syn::Item::Const(item) => &item.attrs,
            syn::Item::Fn(item) => &item.attrs,
            syn::Item::Mod(item) => &item.attrs,
            syn::Item::Use(item) => &item.attrs,
            syn::Item::Impl(item) => &item.attrs,
            syn::Item::Static(item) => &item.attrs,
            _ => {
                syn::visit::visit_item(self, item);
                return;
            }
        };
        if !test_only(attributes) {
            syn::visit::visit_item(self, item);
        }
    }
    fn visit_expr_lit(&mut self, expression: &'ast syn::ExprLit) {
        if let syn::Lit::Str(literal) = &expression.lit {
            self.record(literal.value());
        }
    }
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.tokens(mac.tokens.clone());
    }
}

pub(super) fn literals(source: &str) -> BTreeSet<String> {
    let mut visitor = Literals::default();
    visitor.visit_file(&syn::parse_file(source).expect("valid Rust source"));
    visitor.0
}

fn substitute(template: &str, bindings: &[(&str, &str)]) -> String {
    let mut text = template.to_owned();
    for (field, value) in bindings {
        text = text.replace(field, value);
    }
    // Default operands are opaque data. Sites override numeric/enum operands and
    // optional syntax explicitly; the actual parser still validates the result.
    let mut output = String::new();
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '{' && chars.peek() != Some(&'{') {
            for next in chars.by_ref() {
                if next == '}' {
                    break;
                }
            }
            output.push_str("worker");
        } else {
            output.push(character);
        }
    }
    output
}

pub(super) fn commands(spec: &HintSpec) -> Result<Vec<String>, String> {
    if let Some(reason) = spec.skip {
        if reason.trim().is_empty() {
            return Err("a skipped command needs a reason".into());
        }
        return Ok(Vec::new());
    }
    let text = substitute(spec.template, spec.substitutions);
    let starts: Vec<usize> = text.match_indices("tmt ").map(|(index, _)| index).collect();
    if starts.len() != spec.ends.len() {
        return Err(format!(
            "{} command starts but {} boundaries in {:?}",
            starts.len(),
            spec.ends.len(),
            spec.template
        ));
    }
    starts
        .into_iter()
        .zip(spec.ends)
        .map(|(start, end)| {
            let tail = &text[start..];
            let command = if end.is_empty() {
                tail
            } else {
                tail.split_once(end)
                    .ok_or_else(|| format!("missing command boundary {end:?} in {tail:?}"))?
                    .0
            };
            Ok(command.to_owned())
        })
        .collect()
}

pub(super) fn source_files(root: &Path, output: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(root).expect("source directory") {
        let path = entry.expect("source entry").path();
        let name = path.file_name().unwrap().to_str().unwrap();
        if name == "tests"
            || name == "tests.rs"
            || name.ends_with("_tests.rs")
            || name.starts_with("cli_style")
            || name == "grammar"
            || name == "grammar.rs"
        {
            continue;
        }
        if path.is_dir() {
            source_files(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            output.push(path);
        }
    }
}

#[test]
fn source_coverage_reads_macro_literals_and_ignores_test_fixtures() {
    let source = r#"fn production() { format!("tmt inbox --new-flag"); }
        #[cfg(test)] mod tests { const INVALID: &str = "tmt invalid"; }
        #[cfg(test)] const HINTS: &[&str] = &["tmt copied"];"#;
    assert_eq!(
        literals(source),
        BTreeSet::from(["tmt inbox --new-flag".into()])
    );
    let invalid = HintSpec::core("hint: tmt inbox --new-flag", &[""], &[]);
    let argv = tmt_cli_style::help::ShownExample {
        note: String::new(),
        command: commands(&invalid).unwrap().remove(0),
    }
    .argv()
    .unwrap();
    assert!(super::parse(&argv[1..]).is_err());
}
