//! The #440 driver guard: production code spells a driver's name only in its
//! core descriptor (`tmt-core/src/driver/descriptor.rs`) and its adapter module
//! (`tmt-adapters/src/drivers/`). Everything else iterates the descriptors.

use super::source::{Source, production, production_impl, production_trait};
use proc_macro2::{TokenStream, TokenTree};
use syn::visit::{self, Visit};

/// The only places that may spell a driver's name.
const OWNERS: [(&str, &str); 2] = [
    ("tmt-core", "driver/descriptor.rs"),
    ("tmt-adapters", "drivers/"),
];

pub fn violations(sources: &[Source], names: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for source in sources {
        if OWNERS
            .iter()
            .any(|(package, file)| source.package == *package && source.file.starts_with(file))
        {
            continue;
        }
        let mut visitor = Visitor {
            names,
            found: Vec::new(),
        };
        visitor.visit_file(&source.syntax);
        found.extend(visitor.found.into_iter().map(|name| {
            format!(
                "{}/{}: names the driver {name:?}; iterate tmt_core::driver::ALL instead",
                source.package, source.file
            )
        }));
    }
    found.sort();
    found.dedup();
    found
}

struct Visitor<'a> {
    names: &'a [&'a str],
    found: Vec<String>,
}

impl Visitor<'_> {
    fn literal(&mut self, literal: &syn::Lit) {
        if let syn::Lit::Str(value) = literal {
            let value = value.value();
            if self.names.contains(&value.as_str()) {
                self.found.push(value);
            }
        }
    }

    fn scan(&mut self, tokens: TokenStream) {
        for token in tokens {
            match token {
                TokenTree::Group(group) => self.scan(group.stream()),
                TokenTree::Literal(literal) => self.literal(&syn::Lit::new(literal)),
                _ => {}
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

    fn visit_lit(&mut self, literal: &'ast syn::Lit) {
        self.literal(literal);
        visit::visit_lit(self, literal);
    }

    /// Macro arguments are token streams, not parsed Rust.
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        self.scan(node.tokens.clone());
        visit::visit_macro(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(package: &str, file: &str, text: &str) -> Source {
        Source {
            package: package.into(),
            file: file.into(),
            syntax: syn::parse_file(text).expect("fixture parses"),
        }
    }

    #[test]
    fn a_driver_name_outside_its_module_is_a_violation() {
        let text = r#"
            fn setup() -> [&'static str; 1] { ["alpha"] }
            fn run() { let _ = format!("{}", "alpha"); }
            fn help() -> &'static str { "tmt run worker alpha" }
            #[cfg(test)]
            mod tests { fn fixture() -> &'static str { "alpha" } }
        "#;
        assert_eq!(
            violations(&[source("tmt-cli", "setup_command.rs", text)], &["alpha"]),
            [
                r#"tmt-cli/setup_command.rs: names the driver "alpha"; iterate tmt_core::driver::ALL instead"#
            ]
        );
        assert!(
            violations(
                &[source("tmt-adapters", "drivers/alpha.rs", text)],
                &["alpha"]
            )
            .is_empty()
        );
        assert!(
            violations(
                &[source("tmt-core", "driver/descriptor.rs", text)],
                &["alpha"]
            )
            .is_empty()
        );
        assert_eq!(
            violations(&[source("tmt-core", "drivers/alpha.rs", text)], &["alpha"]).len(),
            1,
            "only the descriptors and the adapter modules may name a driver"
        );
    }
}
