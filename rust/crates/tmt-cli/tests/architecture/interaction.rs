//! The #485 interaction guard: whether a person can see a view or answer a
//! question is decided once, by `tmt_cli_style::Interaction`. The CLI crates
//! never test a handle with `is_terminal` or import `IsTerminal` themselves.
//! `design/cli-style.md` owns the rule; this is its syntactic check.

use super::{
    output::GUARDED,
    source::{Source, production, production_impl, production_trait},
};
use std::collections::BTreeSet;
use syn::visit::{self, Visit};

/// Files that still decide interactivity on their own. Later #485 slices move
/// core, then Office; the list only shrinks.
pub const MIGRATING: &[(&str, &str)] = &[
    ("tmt-cli", "consent.rs"),
    ("tmt-cli", "skill_reminder.rs"),
    ("tmt-office-command", "office_command.rs"),
];

/// Files in the guarded crates that test a terminal themselves.
pub fn findings(sources: &[Source]) -> BTreeSet<(String, String)> {
    let mut found = BTreeSet::new();
    for source in sources
        .iter()
        .filter(|source| GUARDED.contains(&source.package.as_str()))
    {
        let mut visitor = Visitor { hit: false };
        visitor.visit_file(&source.syntax);
        if visitor.hit {
            found.insert((source.package.clone(), source.file.clone()));
        }
    }
    found
}

/// An unlisted file that decides for itself, or a listed file that no longer
/// does and should leave the list.
pub fn violations(sources: &[Source], migrating: &[(&str, &str)]) -> Vec<String> {
    let found = findings(sources);
    let listed: BTreeSet<(String, String)> = migrating
        .iter()
        .map(|(package, file)| ((*package).to_owned(), (*file).to_owned()))
        .collect();
    let mut violations: Vec<String> = found
        .difference(&listed)
        .map(|(package, file)| {
            format!("{package}/{file}: tests a terminal itself; use tmt_cli_style::Interaction")
        })
        .collect();
    violations.extend(listed.difference(&found).map(|(package, file)| {
        format!("{package}/{file}: no longer tests a terminal; remove it from the interaction migration list")
    }));
    violations
}

struct Visitor {
    hit: bool,
}

impl<'ast> Visit<'ast> for Visitor {
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

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "is_terminal" {
            self.hit = true;
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_ident(&mut self, ident: &'ast proc_macro2::Ident) {
        if ident == "IsTerminal" {
            self.hit = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(package: &str, file: &str, text: &str) -> Source {
        Source {
            package: package.into(),
            file: file.into(),
            syntax: syn::parse_file(text).unwrap(),
        }
    }

    #[test]
    fn a_handle_test_outside_the_style_layer_is_found_and_the_list_stays_exact() {
        let sources = [
            source(
                "tmt-ops",
                "board.rs",
                "fn f() -> bool { std::io::stdout().is_terminal() }",
            ),
            source("tmt-ops", "consent.rs", "use std::io::IsTerminal;"),
            source(
                "tmt-ops",
                "tests_only.rs",
                "#[cfg(test)] fn f() -> bool { std::io::stdin().is_terminal() }",
            ),
            source(
                "tmt-cli-style",
                "interaction.rs",
                "fn f() -> bool { std::io::stdin().is_terminal() }",
            ),
            source(
                "tmt-ops",
                "main.rs",
                "fn f(i: Interaction) -> Mode { i.view() }",
            ),
        ];
        assert_eq!(
            violations(
                &sources,
                &[("tmt-ops", "consent.rs"), ("tmt-cli", "gone.rs")]
            ),
            [
                "tmt-ops/board.rs: tests a terminal itself; use tmt_cli_style::Interaction",
                "tmt-cli/gone.rs: no longer tests a terminal; remove it from the interaction migration list",
            ]
        );
    }
}
