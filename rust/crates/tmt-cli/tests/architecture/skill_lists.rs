//! The #440 skill-list guard: `tmt-core/src/skill_catalog.rs` is the one
//! list of bundled skill names. A production array, slice or `vec!` holding two or more catalog
//! names anywhere else is a hand-maintained copy. Single uses are fine.

use super::source::{Source, production, production_impl, production_trait};
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::visit::{self, Visit};

const OWNER: (&str, &str) = ("tmt-core", "skill_catalog.rs");

pub fn violations(sources: &[Source], names: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for source in sources {
        if (source.package.as_str(), source.file.as_str()) == OWNER {
            continue;
        }
        let mut visitor = Visitor { names, lists: 0 };
        visitor.visit_file(&source.syntax);
        if visitor.lists > 0 {
            found.push(format!(
                "{}/{}: lists skill names; derive them from tmt_core::skill_catalog",
                source.package, source.file
            ));
        }
    }
    found
}

struct Visitor<'a> {
    names: &'a [&'a str],
    lists: usize,
}

impl Visitor<'_> {
    fn is_name(&self, literal: &syn::Lit) -> bool {
        matches!(literal, syn::Lit::Str(value) if self.names.contains(&value.value().as_str()))
    }

    fn count(&mut self, names: usize) {
        if names >= 2 {
            self.lists += 1;
        }
    }

    fn literal_names(&self, tokens: &TokenStream) -> usize {
        tokens
            .clone()
            .into_iter()
            .filter(|token| {
                matches!(token, TokenTree::Literal(literal)
                    if self.is_name(&syn::Lit::new(literal.clone())))
            })
            .count()
    }

    /// Bracketed groups inside macro arguments, such as `[..]` in `json!`.
    fn scan(&mut self, tokens: TokenStream) {
        for token in tokens {
            if let TokenTree::Group(group) = token {
                if group.delimiter() == Delimiter::Bracket {
                    let names = self.literal_names(&group.stream());
                    self.count(names);
                }
                self.scan(group.stream());
            }
        }
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

    fn visit_expr_array(&mut self, array: &'ast syn::ExprArray) {
        let names = array
            .elems
            .iter()
            .filter(
                |element| matches!(element, syn::Expr::Lit(literal) if self.is_name(&literal.lit)),
            )
            .count();
        self.count(names);
        visit::visit_expr_array(self, array);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        // `vec![..]` carries its brackets as the macro delimiter.
        if matches!(node.delimiter, syn::MacroDelimiter::Bracket(_)) {
            let names = self.literal_names(&node.tokens);
            self.count(names);
        }
        self.scan(node.tokens.clone());
        visit::visit_macro(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(file: &str, text: &str) -> Source {
        Source {
            package: "tmt-adapters".into(),
            file: file.into(),
            syntax: syn::parse_file(text).expect("fixture parses"),
        }
    }

    #[test]
    fn a_list_of_skill_names_outside_the_catalog_is_a_violation() {
        let names = ["core-skill", "inbox-skill"];
        let listed = r#"
            const CORE: &[&str] = &["core-skill", "inbox-skill"];
        "#;
        let in_macro = r#"fn names() -> Vec<&'static str> { vec!["core-skill", "inbox-skill"] }"#;
        let single = r#"
            fn target() -> &'static str { "core-skill" }
            #[cfg(test)]
            mod tests { const BOTH: [&str; 2] = ["core-skill", "inbox-skill"]; }
        "#;
        for text in [listed, in_macro] {
            assert_eq!(
                violations(&[source("skill_installation/owned.rs", text)], &names),
                [
                    "tmt-adapters/skill_installation/owned.rs: lists skill names; derive them from tmt_core::skill_catalog"
                ]
            );
            let owner = Source {
                package: OWNER.0.into(),
                ..source(OWNER.1, text)
            };
            assert!(violations(&[owner], &names).is_empty());
        }
        assert!(violations(&[source("skill_installation/owned.rs", single)], &names).is_empty());
    }
}
