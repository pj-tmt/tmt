//! The #514 color guard: every color TMT draws is a design token that
//! `tmt_cli_style` renders. Production code elsewhere, the extensions
//! included, names no palette entry or RGB value itself.
//! `design/cli-style.md` owns the rule; this is its syntactic check.

use super::source::{Source, production, production_impl, production_trait};
use std::collections::BTreeSet;
use syn::visit::{self, Visit};

/// The crate that owns every color.
const OWNER: &str = "tmt-cli-style";

/// anstyle's color types; any use names a color.
const COLOR_TYPES: &[&str] = &["AnsiColor", "Ansi256Color", "RgbColor"];

/// One finding per file that names a color outside the owner.
pub fn violations(sources: &[Source]) -> Vec<String> {
    let mut found = BTreeSet::new();
    for source in sources.iter().filter(|source| source.package != OWNER) {
        let mut visitor = Visitor { hit: None };
        visitor.visit_file(&source.syntax);
        if let Some(name) = visitor.hit {
            found.insert(format!(
                "{}/{}: names the color {name}; use a tmt_cli_style::Role or Token",
                source.package, source.file
            ));
        }
    }
    found.into_iter().collect()
}

struct Visitor {
    hit: Option<String>,
}

impl Visitor {
    fn found(&mut self, name: String) {
        self.hit.get_or_insert(name);
    }
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

    /// `Color::Red`, `Color::Rgb(..)`, `Color::Indexed(..)`: a ratatui or
    /// anstyle color value. `Color::Reset` is the terminal's own color, not
    /// a choice of one.
    fn visit_path(&mut self, path: &'ast syn::Path) {
        let segments: Vec<String> = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        for pair in segments.windows(2) {
            if pair[0] == "Color" && pair[1] != "Reset" {
                self.found(pair.join("::"));
            }
        }
        visit::visit_path(self, path);
    }

    fn visit_ident(&mut self, ident: &'ast proc_macro2::Ident) {
        if COLOR_TYPES.iter().any(|name| ident == name) {
            self.found(ident.to_string());
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
    fn colors_are_named_only_by_the_style_crate_and_tests() {
        let sources = [
            source(
                "tmt-ops",
                "view.rs",
                "fn f() -> Style { Style::new().fg(Color::Magenta) }",
            ),
            source(
                "tmt-ops",
                "rgb.rs",
                "fn f() -> Color { ratatui::style::Color::Rgb(1, 2, 3) }",
            ),
            source("tmt-cli", "hue.rs", "use tmt_cli_style::AnsiColor;"),
            source(
                "tmt-ops",
                "reset.rs",
                "fn f(c: Cell) -> bool { c.fg == Color::Reset }",
            ),
            source(
                "tmt-ops",
                "tests_only.rs",
                "#[cfg(test)] mod tests { fn f() -> Color { Color::Red } }",
            ),
            source(
                "tmt-cli-style",
                "theme.rs",
                "const RED: AnsiColor = AnsiColor::Red;",
            ),
            source(
                "tmt-ops",
                "look.rs",
                "fn f(look: Look) -> Style { look.role(Role::Blocked) }",
            ),
        ];
        assert_eq!(
            violations(&sources),
            [
                "tmt-cli/hue.rs: names the color AnsiColor; use a tmt_cli_style::Role or Token",
                "tmt-ops/rgb.rs: names the color Color::Rgb; use a tmt_cli_style::Role or Token",
                "tmt-ops/view.rs: names the color Color::Magenta; use a tmt_cli_style::Role or Token",
            ]
        );
    }
}
