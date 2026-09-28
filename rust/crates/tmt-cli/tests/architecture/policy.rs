//! Review policy, separate from syntax collection and adverse examples.

use super::source::{Source, production, production_impl, production_trait};
use serde_json::Value;
use std::collections::BTreeMap;
use syn::{
    Item, UseTree,
    visit::{self, Visit},
};

// These are reviewed layer permissions, not a second version/dependency graph.
// Cargo metadata is the inventory, including renamed and target/build entries.
pub fn dependency_violations(package: &Value) -> Vec<String> {
    let name = package["name"].as_str().expect("Cargo package name");
    let allowed: &[&str] = match name {
        "tmt-core" => &[
            "semver",
            "uuid",
            "icu_casemap",
            "icu_normalizer",
            "icu_locale_core",
            "sha2",
        ],
        "tmt-office-model" => &[
            "tmt-core",
            "serde",
            "serde_json",
            "base64",
            "png",
            "url",
            "sha2",
            "uuid",
            "semver",
        ],
        "tmt-adapters" => &[
            "tmt-office-model",
            "getrandom",
            "keyring-core",
            "zbus-secret-service-keyring-store",
            "apple-native-keyring-store",
            "serde",
            "url",
            "ureq",
            "semver",
            "tar",
            "flate2",
            "tmt-core",
            "rusqlite",
            "serde_json",
            "base64",
            "uuid",
            "subprocess",
            "nix",
            "signal-hook",
            "sha2",
        ],
        "tmt-office" => &[
            "tmt-office-model",
            "tmt-office-command",
            "tmt-office-storage",
            "tmt-core",
            "tmt-adapters",
            "base64",
            "getrandom",
            "httparse",
            "serde",
            "serde_json",
            "uuid",
        ],
        "tmt-cli" => &[
            "tmt-office-command",
            "tmt-command-output",
            "tmt-core",
            "tmt-adapters",
            "clap",
            "clap_complete",
            "serde_json",
        ],
        "tmt-command-output" => &["tmt-core", "tmt-adapters", "serde_json", "unicode-width"],
        "tmt-office-command" => &[
            "tmt-core",
            "tmt-office-model",
            "tmt-adapters",
            "tmt-command-output",
            "clap",
            "serde_json",
        ],
        // Office-owned storage reaches core only through the public config and
        // file-lock owners; it reads the core database directly only to migrate.
        "tmt-office-storage" => &[
            "tmt-adapters",
            "tmt-core",
            "tmt-office-model",
            "rusqlite",
            "base64",
            "serde",
            "sha2",
            "serde_json",
            "uuid",
        ],
        // Squad is a public-interface consumer: it reaches TMT only through
        // commands and `tmt api`, so it may depend on no workspace crate.
        "tmt-squad" => &[
            "clap",
            "serde_json",
            "toml_edit",
            "subprocess",
            "ratatui",
            "unicode-width",
            "signal-hook",
        ],
        _ => return vec![format!("unreviewed workspace package {name}")],
    };
    package["dependencies"]
        .as_array()
        .expect("Cargo dependencies")
        .iter()
        .filter(|d| d["kind"] != "dev")
        .filter_map(|d| {
            let dependency = d["name"].as_str().expect("Cargo dependency name");
            // Source paths use canonical crate names. Renaming even an allowed
            // package requires an explicit policy review instead of bypassing
            // the source-layer checks through a new external crate alias.
            let unreviewed = !allowed.contains(&dependency) || !d["rename"].is_null();
            unreviewed.then(|| {
                format!(
                    "{name}: unreviewed production dependency {dependency} (kind={}, target={}, rename={})",
                    d["kind"], d["target"], d["rename"]
                )
            })
        })
        .collect()
}

struct Declaration {
    name: String,
    public: bool,
    function: bool,
    nested: bool,
}
#[derive(Default)]
struct Facts {
    references: Vec<Vec<String>>,
    reexports: Vec<Vec<String>>,
    declarations: Vec<Declaration>,
    broad_failure: bool,
    depth: usize,
}

fn use_paths(tree: &UseTree, mut prefix: Vec<String>, paths: &mut Vec<Vec<String>>) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            use_paths(&path.tree, prefix, paths);
        }
        UseTree::Name(name) => {
            prefix.push(name.ident.to_string());
            paths.push(prefix);
        }
        UseTree::Rename(name) => {
            prefix.push(name.ident.to_string());
            paths.push(prefix);
        }
        UseTree::Glob(_) => {
            prefix.push("*".into());
            paths.push(prefix);
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                use_paths(tree, prefix.clone(), paths);
            }
        }
    }
}

fn type_named(value: &syn::Type, name: &str) -> bool {
    matches!(value, syn::Type::Path(path)
        if path.path.segments.last().is_some_and(|segment| segment.ident == name))
}

impl<'ast> Visit<'ast> for Facts {
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

    fn visit_item(&mut self, item: &'ast Item) {
        if !production(item) {
            return;
        }
        let declaration = match item {
            Item::Struct(i) => Some((&i.ident, &i.vis, false)),
            Item::Enum(i) => Some((&i.ident, &i.vis, false)),
            Item::Type(i) => Some((&i.ident, &i.vis, false)),
            Item::Trait(i) => Some((&i.ident, &i.vis, false)),
            Item::TraitAlias(i) => Some((&i.ident, &i.vis, false)),
            Item::Union(i) => Some((&i.ident, &i.vis, false)),
            Item::Fn(i) => Some((&i.sig.ident, &i.vis, true)),
            _ => None,
        };
        if let Some((ident, visibility, function)) = declaration {
            self.declarations.push(Declaration {
                name: ident.to_string(),
                public: matches!(visibility, syn::Visibility::Public(_)),
                function,
                nested: self.depth != 0,
            });
        }
        self.depth += 1;
        visit::visit_item(self, item);
        self.depth -= 1;
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        use_paths(&item.tree, Vec::new(), &mut self.references);
        if matches!(item.vis, syn::Visibility::Public(_)) {
            use_paths(&item.tree, Vec::new(), &mut self.reexports);
        }
    }

    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        self.references.push(vec![item.ident.to_string()]);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.references
            .push(path.segments.iter().map(|p| p.ident.to_string()).collect());
        visit::visit_path(self, path);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if let Some((_, path, _)) = &item.trait_
            && let Some(segment) = path.segments.last()
            && segment.ident == "From"
            && let syn::PathArguments::AngleBracketed(args) = &segment.arguments
        {
            let string = args.args.iter().any(|arg| {
                matches!(arg, syn::GenericArgument::Type(value) if type_named(value, "String"))
            });
            self.broad_failure |= string && type_named(&item.self_ty, "Failure");
        }
        visit::visit_item_impl(self, item);
    }
}

fn owns_declarations(source: &Source) -> bool {
    source.package == "tmt-core"
        || (source.package == "tmt-office-model" && source.file.starts_with("office_"))
        || (source.package == "tmt-cli" && source.file == "invocation.rs")
        || (source.package == "tmt-command-output" && source.file == "lib.rs")
}

// Retained consumers while Office storage and commands are extracted. These are
// exact production paths, not permission for new core-to-Office coupling.
fn office_consumer(source: &Source) -> bool {
    let allowed: &[&str] = match source.package.as_str() {
        "tmt-adapters" => &[
            "office_avatar.rs",
            "office_block.rs",
            "office_board.rs",
            "office_companion.rs",
            "office_companion/extension.rs",
            "office_companion/whiteboard.rs",
            "office_companion/world.rs",
            "office_pairing.rs",
            "office_pairing/hooks.rs",
            "office_pairing/invocation.rs",
            "office_pairing/record.rs",
            "office_pairing/remote/blocks.rs",
            "office_profile.rs",
            "office_prop.rs",
            "office_service.rs",
            "office_whiteboard/export.rs",
            "office_world.rs",
            "office_world/reply.rs",
            "repository_remote.rs",
        ],
        "tmt-cli" => &[],
        "tmt-office-command" => return true,
        // Office-owned storage and its companion operations.
        "tmt-office-storage" => return true,
        _ => &[],
    };
    allowed.contains(&source.file.as_str())
}

pub fn source_violations(sources: &[Source]) -> Vec<String> {
    let facts: Vec<_> = sources
        .iter()
        .map(|source| {
            let mut facts = Facts::default();
            facts.visit_file(&source.syntax);
            facts
        })
        .collect();
    let mut owners = BTreeMap::new();
    let mut violations = Vec::new();
    for (source, facts) in sources.iter().zip(&facts) {
        if !owns_declarations(source) {
            continue;
        }
        for d in &facts.declarations {
            // Public free functions in core are policy entrypoints. Methods and
            // command-local execute/run helpers do not become reserved names.
            if !d.public
                || (d.function
                    && !["tmt-core", "tmt-office-model"].contains(&source.package.as_str()))
            {
                continue;
            }
            let owner = format!("{}/{}", source.package, source.file);
            match owners.entry((d.name.clone(), d.function)) {
                std::collections::btree_map::Entry::Occupied(previous) => {
                    violations.push(format!(
                        "{owner}: duplicate owner {} (already {})",
                        d.name,
                        previous.get()
                    ));
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(owner);
                }
            }
        }
    }
    for (source, facts) in sources.iter().zip(&facts) {
        let location = format!("{}/{}", source.package, source.file);
        if ["tmt-core", "tmt-adapters", "tmt-cli"].contains(&source.package.as_str()) {
            for path in &facts.reexports {
                if path.first().is_some_and(|root| root == "tmt_office_model") {
                    violations.push(format!(
                        "{location}: import the Office owner directly; do not re-export {}",
                        path.join("::")
                    ));
                }
            }
        }
        for d in &facts.declarations {
            // Public owners (including inline modules) were checked above. Keep one actionable
            // diagnostic per competing owner instead of reporting both ways.
            if owns_declarations(source)
                && d.public
                && (!d.function
                    || ["tmt-core", "tmt-office-model"].contains(&source.package.as_str()))
            {
                continue;
            }
            if let Some(owner) = owners.get(&(d.name.clone(), d.function))
                && (owner != &location || d.nested)
            {
                violations.push(format!("{location}: {} is owned by {owner}", d.name));
            }
        }
        for path in &facts.references {
            let root = path.first().map(String::as_str).unwrap_or_default();
            let module = path.get(1).map(String::as_str).unwrap_or_default();
            let model = source.package == "tmt-office-model";
            if source.package == "tmt-core" || model {
                let pure_std = [
                    "borrow",
                    "boxed",
                    "char",
                    "clone",
                    "cmp",
                    "collections",
                    "convert",
                    "default",
                    "error",
                    "fmt",
                    "hash",
                    "iter",
                    "marker",
                    "mem",
                    "num",
                    "ops",
                    "option",
                    "result",
                    "slice",
                    "str",
                    "string",
                    "vec",
                ];
                let model_value = model
                    && ((module == "sync" && path.get(2).is_some_and(|p| p == "OnceLock"))
                        || (module == "io"
                            && path
                                .get(2)
                                .is_some_and(|p| ["Error", "Cursor"].contains(&p.as_str()))));
                if (root == "std" && !pure_std.contains(&module) && !model_value)
                    || ["print", "println", "eprint", "eprintln", "dbg"].contains(&root)
                {
                    violations.push(format!(
                        "{location}: non-pure core reference {}",
                        path.join("::")
                    ));
                }
            }
            if model
                && root == "tmt_core"
                && !["dispatch", "limits", "content_digest"].contains(&module)
            {
                violations.push(format!(
                    "{location}: Office model cannot acquire core runtime responsibilities via {}",
                    path.join("::")
                ));
            }
            if root == "tmt_office_model"
                && source.package != "tmt-office"
                && !office_consumer(source)
            {
                violations.push(format!(
                    "{location}: unreviewed Office dependency {}",
                    path.join("::")
                ));
            }
            if source.package == "tmt-squad" && root.starts_with("tmt_") {
                violations.push(format!(
                    "{location}: squad reaches TMT only through public commands, not {}",
                    path.join("::")
                ));
            }
            if root == "tmt_squad" && source.package != "tmt-squad" {
                violations.push(format!(
                    "{location}: no package may depend on the squad extension: {}",
                    path.join("::")
                ));
            }
            if root == "tmt_office_command"
                && ["tmt-core", "tmt-adapters", "tmt-cli"].contains(&source.package.as_str())
                && !(source.package == "tmt-cli" && source.file == "office_facade.rs")
            {
                violations.push(format!(
                    "{location}: Office commands must be reached through the reserved facade: {}",
                    path.join("::")
                ));
            }
            if source.package == "tmt-cli" {
                let grammar_owner =
                    source.file == "grammar.rs" || source.file.starts_with("grammar/");
                let parsing = grammar_owner
                    || ["parser.rs", "diagnostics.rs", "invocation.rs"]
                        .contains(&source.file.as_str());
                let office_registration = module == "office_facade"
                    && path.get(2).is_some_and(|part| {
                        ["grammar", "parser", "invocation"].contains(&part.as_str())
                    });
                if parsing
                    && (root == "tmt_adapters"
                        || (["crate", "super"].contains(&root)
                            && !office_registration
                            && !["grammar", "parser", "diagnostics", "invocation"]
                                .contains(&module)))
                {
                    violations.push(format!(
                        "{location}: parsing depends on effects via {}",
                        path.join("::")
                    ));
                }
                if ["clap", "clap_complete"].contains(&root)
                    && !grammar_owner
                    && (!["main.rs", "parser.rs", "diagnostics.rs"].contains(&source.file.as_str()))
                {
                    violations.push(format!(
                        "{location}: CLI parsing belongs to grammar/parser, not {}",
                        path.join("::")
                    ));
                }
            }
        }
        if facts.broad_failure {
            violations.push(format!("{location}: map String errors explicitly; do not implement From<String> for shared Failure"));
        }
    }
    violations.sort();
    violations.dedup();
    violations
}
