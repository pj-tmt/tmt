//! The #501 extension guard: no extension source, production or test, names
//! core's terminal-host port or its binding and endpoint model. Extensions
//! reach presence through `tmt ls --json` and the caller through
//! `tmt whoami`, so host changes (#479) stay inside core. Unlike the other
//! guards this one reads test code too: a test stand-in that links the host
//! port breaks with every host change just the same.

use std::{fs, path::Path};
use syn::visit::{self, Visit};

const FORBIDDEN: [&str; 6] = [
    "tmt_adapters::herdr",
    "tmt_adapters::host",
    "tmt_adapters::tmux",
    "tmt_core::binding",
    "tmt_core::endpoint",
    "tmt_core::host",
];

/// Every Rust file under `<extensions>/*/rust`, as `(relative path, text)`.
pub fn sources(extensions: &Path) -> Vec<(String, String)> {
    fn walk(directory: &Path, root: &Path, found: &mut Vec<(String, String)>) {
        let mut entries: Vec<_> = fs::read_dir(directory)
            .expect("read extension source directory")
            .map(|entry| entry.expect("extension directory entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, root, found);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let relative = path.strip_prefix(root).unwrap().display().to_string();
                found.push((relative, fs::read_to_string(&path).expect("read source")));
            }
        }
    }
    let mut found = Vec::new();
    for extension in fs::read_dir(extensions).expect("read extensions") {
        let rust = extension.expect("extension entry").path().join("rust");
        if rust.is_dir() {
            walk(&rust, extensions, &mut found);
        }
    }
    found
}

pub fn violations(sources: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    for (file, text) in sources {
        let syntax = match syn::parse_file(text) {
            Ok(syntax) => syntax,
            Err(error) => {
                violations.push(format!("extensions/{file}: does not parse: {error}"));
                continue;
            }
        };
        let mut paths = Paths::default();
        paths.visit_file(&syntax);
        for path in paths.found {
            if let Some(forbidden) = FORBIDDEN.iter().find(|forbidden| {
                path == **forbidden || path.starts_with(&format!("{forbidden}::"))
            }) {
                violations.push(format!(
                    "extensions/{file}: names {forbidden}; reach core through tmt ls --json or tmt whoami"
                ));
            }
        }
    }
    violations.sort();
    violations.dedup();
    violations
}

/// Every path written in the file: `use` trees flattened, plus expression,
/// type and macro paths.
#[derive(Default)]
struct Paths {
    found: Vec<String>,
}

fn flatten(prefix: &str, tree: &syn::UseTree, found: &mut Vec<String>) {
    let join = |name: String| {
        if prefix.is_empty() {
            name
        } else {
            format!("{prefix}::{name}")
        }
    };
    match tree {
        syn::UseTree::Path(path) => flatten(&join(path.ident.to_string()), &path.tree, found),
        syn::UseTree::Name(name) => found.push(join(name.ident.to_string())),
        syn::UseTree::Rename(rename) => found.push(join(rename.ident.to_string())),
        syn::UseTree::Glob(_) => found.push(prefix.to_owned()),
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                flatten(prefix, tree, found);
            }
        }
    }
}

impl Visit<'_> for Paths {
    fn visit_item_use(&mut self, item: &syn::ItemUse) {
        flatten("", &item.tree, &mut self.found);
    }

    fn visit_path(&mut self, path: &syn::Path) {
        self.found.push(
            path.segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>()
                .join("::"),
        );
        visit::visit_path(self, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(text: &str) -> Vec<String> {
        violations(&[("tmt-x/rust/x/src/lib.rs".into(), text.into())])
    }

    #[test]
    fn host_and_binding_paths_are_violations_in_production_and_tests() {
        for text in [
            "use tmt_adapters::host::Host;",
            "use tmt_adapters::{api, host::{CallerEnvironment, Host}};",
            "use tmt_adapters::host;",
            "use tmt_core::{binding, room::RoomRepository};",
            "use tmt_core::endpoint::*;",
            "fn f() { let _ = tmt_core::host::HostKind::Tmux; }",
            "#[cfg(test)] mod tests { fn f() { tmt_adapters::tmux::Tmux::default(); } }",
        ] {
            assert_eq!(check(text).len(), 1, "{text}");
        }
    }

    #[test]
    fn other_core_paths_and_lookalikes_are_not_this_guards_concern() {
        for text in [
            "use tmt_adapters::{api, config::ConfigPaths, room::encode_rooms};",
            "use tmt_core::{identity::IdentityReader, room::RoomRepository};",
            "use tmt_adapters::hostname::lookup;",
            "fn f() -> &'static str { \"tmt_core::binding\" }",
        ] {
            assert!(check(text).is_empty(), "{text}");
        }
    }
}
