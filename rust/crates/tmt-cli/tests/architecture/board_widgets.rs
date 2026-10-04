//! Squad consumes tmt-tui drawing primitives. The remaining raw widget uses
//! are exact file/widget exceptions until #1544 migrates those surfaces.

use super::source::{Source, production, production_impl, production_trait};
use proc_macro2::{TokenStream, TokenTree};
use std::collections::{BTreeMap, BTreeSet};
use syn::visit::{self, Visit};

pub struct Exception {
    pub file: &'static str,
    pub widget: &'static str,
    pub reason: &'static str,
    pub issue: &'static str,
}

const FOLLOW_UP: &str = "https://github.com/pj-tmt/tmt/issues/1544";

macro_rules! exception {
    ($file:literal, $widget:literal, $reason:literal) => {
        Exception {
            file: $file,
            widget: $widget,
            reason: $reason,
            issue: FOLLOW_UP,
        }
    };
}

/// No file-wide exemption: a new widget in a listed file still fails.
pub const EXCEPTIONS: &[Exception] = &[
    exception!(
        "board/view.rs",
        "Paragraph",
        "Preserve frozen tab and summary strip painting."
    ),
    exception!(
        "board/view/header.rs",
        "Paragraph",
        "Preserve the existing right-aligned meter band."
    ),
    exception!(
        "board/view/footer.rs",
        "Paragraph",
        "Preserve footer hints, notices and composer text."
    ),
    exception!(
        "board/view/panes.rs",
        "Block",
        "Preserve existing pane borders and title geometry."
    ),
    exception!(
        "board/view/panes.rs",
        "Borders",
        "Preserve existing pane border configuration."
    ),
    exception!(
        "board/view/panes.rs",
        "Paragraph",
        "Preserve folded titles, pane tabs and layout errors."
    ),
    exception!(
        "board/view/rows.rs",
        "Paragraph",
        "Preserve loading and row-admission error text."
    ),
    exception!(
        "board/view/detail.rs",
        "Paragraph",
        "Preserve the empty selected-member placeholder."
    ),
    exception!(
        "board/view/overlays.rs",
        "Clear",
        "Preserve the existing opaque action-menu area."
    ),
    exception!(
        "board/view/overlays.rs",
        "Block",
        "Preserve the existing action-menu chrome."
    ),
    exception!(
        "board/view/overlays.rs",
        "Borders",
        "Preserve existing action-menu border configuration."
    ),
    exception!(
        "board/view/overlays.rs",
        "Paragraph",
        "Preserve existing action-menu entry painting."
    ),
    exception!(
        "board/scroll.rs",
        "Paragraph",
        "Preserve visible-line decoration and overflow indicators."
    ),
];

type Finding = (String, String);
type Aliases = BTreeMap<String, BTreeSet<Vec<String>>>;

fn imports(tree: &syn::UseTree, prefix: &[String], out: &mut Vec<(Vec<String>, String)>) {
    let mut path = prefix.to_vec();
    match tree {
        syn::UseTree::Path(item) => {
            path.push(item.ident.to_string());
            imports(&item.tree, &path, out);
        }
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                imports(tree, prefix, out);
            }
        }
        syn::UseTree::Name(name) => {
            if name.ident != "self" {
                path.push(name.ident.to_string());
            }
            if let Some(alias) = path.last() {
                out.push((path.clone(), alias.clone()));
            }
        }
        syn::UseTree::Rename(rename) => {
            if rename.ident != "self" {
                path.push(rename.ident.to_string());
            }
            out.push((path, rename.rename.to_string()));
        }
        syn::UseTree::Glob(_) => {
            path.push("*".into());
            out.push((path, "*".into()));
        }
    }
}

#[derive(Default)]
struct Visitor {
    aliases: Aliases,
    widgets: BTreeSet<String>,
}

impl Visitor {
    fn path(&mut self, path: &[String], seen: &mut BTreeSet<String>) {
        let Some(root) = path.first() else { return };
        if root == "ratatui" {
            match path.get(1).map(String::as_str) {
                Some("widgets") => {
                    if let Some(widget) = path.get(2) {
                        // Traits and state are infrastructure, not drawing primitives.
                        if !matches!(
                            widget.as_str(),
                            "Widget"
                                | "WidgetRef"
                                | "StatefulWidget"
                                | "StatefulWidgetRef"
                                | "BlockExt"
                                | "ListState"
                                | "TableState"
                                | "ScrollbarState"
                        ) {
                            self.widgets.insert(widget.clone());
                        }
                    }
                }
                // A root glob hides where `widgets` came from; fail closed.
                Some("*") => {
                    self.widgets.insert("*".into());
                }
                _ => {}
            }
            return;
        }
        if !seen.insert(root.clone()) {
            return;
        }
        if let Some(targets) = self.aliases.get(root).cloned() {
            for mut target in targets {
                target.extend_from_slice(&path[1..]);
                self.path(&target, seen);
            }
        }
        seen.remove(root);
    }

    /// Macros are not expanded; inspect their path tokens, never string data.
    fn tokens(&mut self, tokens: TokenStream) {
        let tokens: Vec<_> = tokens.into_iter().collect();
        for (index, token) in tokens.iter().enumerate() {
            match token {
                TokenTree::Group(group) => self.tokens(group.stream()),
                TokenTree::Ident(ident) => {
                    let mut path = vec![ident.to_string()];
                    let mut tail = index + 1;
                    while let [
                        TokenTree::Punct(a),
                        TokenTree::Punct(b),
                        TokenTree::Ident(next),
                        ..,
                    ] = &tokens[tail..]
                    {
                        if a.as_char() != ':' || b.as_char() != ':' {
                            break;
                        }
                        path.push(next.to_string());
                        tail += 3;
                    }
                    self.path(&path, &mut BTreeSet::new());
                }
                _ => {}
            }
        }
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
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        let mut paths = Vec::new();
        imports(&item.tree, &[], &mut paths);
        for (path, alias) in paths {
            self.path(&path, &mut BTreeSet::new());
            if alias != "*" {
                self.aliases.entry(alias).or_default().insert(path);
            }
        }
    }
    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        if item.ident == "ratatui" {
            let alias = item.rename.as_ref().map_or(&item.ident, |(_, alias)| alias);
            self.aliases
                .entry(alias.to_string())
                .or_default()
                .insert(vec!["ratatui".into()]);
        }
    }
    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.path(
            &path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect::<Vec<_>>(),
            &mut BTreeSet::new(),
        );
        visit::visit_path(self, path);
    }
    fn visit_macro(&mut self, item: &'ast syn::Macro) {
        self.tokens(item.tokens.clone());
    }
}

// Per-file aliases do not resolve crate-local re-exports across modules.
fn findings(sources: &[Source]) -> BTreeSet<Finding> {
    let mut found = BTreeSet::new();
    for source in sources.iter().filter(|s| s.package == "tmt-squad") {
        let mut visitor = Visitor::default();
        // Resolve aliases even when imports occur after their uses. Two walks
        // retain the same fail-closed production cfg rules as source discovery.
        visitor.visit_file(&source.syntax);
        visitor.visit_file(&source.syntax);
        found.extend(
            visitor
                .widgets
                .into_iter()
                .map(|widget| (source.file.clone(), widget)),
        );
    }
    found
}

pub fn violations(sources: &[Source], exceptions: &[Exception]) -> Vec<String> {
    let found = findings(sources);
    let listed: BTreeSet<_> = exceptions
        .iter()
        .map(|e| (e.file.to_owned(), e.widget.to_owned()))
        .collect();
    let mut violations: Vec<_> = found
        .difference(&listed)
        .map(|(file, widget)| {
            format!("tmt-squad/{file}: raw ratatui widget {widget}; use tmt-tui drawing primitives")
        })
        .collect();
    violations.extend(listed.difference(&found).map(|(file, widget)| {
        format!("tmt-squad/{file}: no raw {widget} remains; remove its board-widget exception")
    }));
    let mut unique = BTreeSet::new();
    for exception in exceptions {
        if exception.reason.trim().is_empty()
            || exception.issue != FOLLOW_UP
            || exception.widget == "*"
            || !unique.insert((exception.file, exception.widget))
        {
            violations.push(format!(
                "tmt-squad/{}: {} exception needs a unique exact widget, reason and #1544 link",
                exception.file, exception.widget
            ));
        }
    }
    violations
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
    fn qualified_grouped_aliased_and_macro_widgets_are_found() {
        for text in [
            "fn paint() { ratatui::widgets::Paragraph::new(\"text\"); }",
            "use ratatui::{widgets::{Paragraph as Raw}}; fn paint() { Raw::new(\"text\"); }",
            "use ratatui as ui; use ui::widgets as w; fn paint() { w::Paragraph::new(\"text\"); }",
            "fn paint() { ui::widgets::Paragraph::new(\"text\"); } use ratatui as ui;",
            "extern crate ratatui as ui; fn paint() { ui::widgets::Paragraph::new(\"text\"); }",
            "fn paint() { wrap!(ratatui::widgets::Paragraph::new(\"text\")); }",
            "#[cfg(any(test, unix))] fn paint() { ratatui::widgets::Paragraph::new(\"text\"); }",
        ] {
            assert_eq!(
                findings(&[source("tmt-squad", "board/seeded.rs", text)]),
                BTreeSet::from([("board/seeded.rs".into(), "Paragraph".into())]),
                "{text}"
            );
        }
        assert!(
            !violations(
                &[source(
                    "tmt-squad",
                    "board/seeded.rs",
                    "use ratatui::widgets::*;"
                )],
                &[]
            )
            .is_empty()
        );
        assert!(
            !violations(
                &[source("tmt-squad", "board/seeded.rs", "use ratatui::*;")],
                &[]
            )
            .is_empty()
        );
    }

    #[test]
    fn drawing_owner_infrastructure_tests_and_unrelated_names_are_accepted() {
        let sources = [
            source(
                "tmt-tui",
                "paint.rs",
                "use ratatui::widgets::{Paragraph, Block, Clear, List, Table};",
            ),
            source(
                "tmt-squad",
                "board/paint.rs",
                "fn paint() { tmt_tui::paint::paint(); } use ratatui::widgets::{Widget, ListState};",
            ),
            source(
                "tmt-squad",
                "status.rs",
                "use tmt_cli_style::table::Table; fn list() { Table::new(); }",
            ),
            source(
                "tmt-squad",
                "config.rs",
                "use toml_edit::Table; fn edit() { Table::new(); }",
            ),
            source(
                "tmt-squad",
                "board/tests.rs",
                "#[cfg(test)] mod tests { use ratatui::widgets::Paragraph; }",
            ),
            source(
                "tmt-squad",
                "board/help.rs",
                "fn help() { hint!(\"ratatui::widgets::Paragraph\"); }",
            ),
        ];
        assert!(violations(&sources, &[]).is_empty());
    }

    #[test]
    fn exceptions_are_exact_documented_and_removed_when_stale() {
        let exceptions = [Exception {
            file: "board/paint.rs",
            widget: "Paragraph",
            reason: "Preserve existing frozen paint.",
            issue: FOLLOW_UP,
        }];
        let sources = [source(
            "tmt-squad",
            "board/paint.rs",
            "use ratatui::widgets::{Paragraph, Table};",
        )];
        assert_eq!(
            violations(&sources, &exceptions),
            ["tmt-squad/board/paint.rs: raw ratatui widget Table; use tmt-tui drawing primitives"]
        );
        assert_eq!(
            violations(&[], &exceptions),
            [
                "tmt-squad/board/paint.rs: no raw Paragraph remains; remove its board-widget exception"
            ]
        );
        for (reason, issue, widget) in [
            ("", FOLLOW_UP, "Paragraph"),
            ("reason", "", "Paragraph"),
            ("reason", FOLLOW_UP, "*"),
        ] {
            let invalid = [Exception {
                reason,
                issue,
                widget,
                ..exceptions[0]
            }];
            assert!(
                violations(&sources, &invalid)
                    .iter()
                    .any(|v| v.contains("needs a unique exact widget"))
            );
        }
    }

    #[test]
    fn seeded_raw_widget_fails_through_real_module_discovery() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = loop {
            let candidate = std::env::temp_dir().join(format!(
                "tmt-board-widget-guard-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create board-widget fixture: {error}"),
            }
        };
        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let fixture = Fixture(root);
        std::fs::create_dir(fixture.0.join("board")).unwrap();
        let entry = fixture.0.join("main.rs");
        let seeded = fixture.0.join("board/seeded.rs");
        std::fs::write(&entry, "mod board;").unwrap();
        std::fs::write(fixture.0.join("board/mod.rs"), "mod seeded;").unwrap();
        std::fs::write(&seeded, "fn paint() { tmt_tui::paint::paint(); }").unwrap();
        assert!(
            violations(
                &super::super::source::collect("tmt-squad", &entry).unwrap(),
                &[]
            )
            .is_empty()
        );
        std::fs::write(
            &seeded,
            "fn paint() { ratatui::widgets::Paragraph::new(\"seeded\"); }",
        )
        .unwrap();
        assert_eq!(
            violations(
                &super::super::source::collect("tmt-squad", &entry).unwrap(),
                &[]
            ),
            [
                "tmt-squad/board/seeded.rs: raw ratatui widget Paragraph; use tmt-tui drawing primitives"
            ]
        );
    }
}
