//! Review policy, separate from syntax collection and adverse examples.

use super::source::{Source, production, production_impl, production_trait};
use serde_json::Value;
use std::collections::BTreeMap;
use syn::{
    Item, UseTree,
    visit::{self, Visit},
};

// Exact dev edges: one reviewed row with its fixture reason per dependency.
// Versions/features remain Cargo-owned; aliases require canonical crate names.
// tmt-invoke retains its stricter all-kind leaf policy below.
const DEV_DEPENDENCIES: &[(&str, &str, Option<&str>)] = &[
    ("tmt-adapters", "tmt-office-model", None), // storage migration model fixtures
    ("tmt-adapters", "nix", Some("cfg(unix)")), // subprocess and pipe fixtures
    ("tmt-adapters", "rcgen", Some("cfg(unix)")), // release HTTP TLS certificates
    ("tmt-adapters", "rustls", Some("cfg(unix)")), // release HTTP TLS server
    ("tmt-cli", "crossterm", None),             // binding presentation fixtures
    ("tmt-cli", "insta", None),                 // command rendering snapshots
    ("tmt-cli", "proc-macro2", None),           // architecture syntax fixtures
    ("tmt-cli", "toml_edit", None),             // audited unsafe-boundary manifest policy
    ("tmt-cli", "syn", None),                   // architecture AST checks
    ("tmt-cli", "tmt-office-model", None),      // Office parser fixtures
    ("tmt-cli", "nix", Some("cfg(unix)")),      // stdin signal and observer readiness fixtures
    ("tmt-cli", "rusqlite", Some("cfg(unix)")), // request-observer SQL oracle
    ("tmt-cli-style", "crossterm", None),       // table terminal-style assertions
    ("tmt-cli-style", "insta", None),           // rendering snapshots
    ("tmt-cli-style", "serde_json", None),      // theme serialization assertions
    ("tmt-squad", "tmt-tui", None),             // test-scoped markup/source parity adapter
    ("tmt-office", "png", None),                // whiteboard image fixtures
    ("tmt-office", "rusqlite", None),           // whiteboard and world SQL oracles
    ("tmt-office", "tmt-office-storage", None), // in-process props fixtures
    ("tmt-office-command", "flate2", None),     // compressed release archive fixtures
    ("tmt-office-command", "png", None),        // whiteboard image fixtures
    ("tmt-office-command", "tar", None),        // release archive fixtures
    ("tmt-office-storage", "png", None),        // stored image fixtures
];

// These are reviewed layer permissions, not a second version/dependency graph.
// Cargo metadata is the inventory, including renamed and target/build entries.
pub fn dependency_violations(package: &Value) -> Vec<String> {
    let name = package["name"].as_str().expect("Cargo package name");
    let allowed: &[&str] = match name {
        // The host grammar is core's one workspace dependency: pure syntax,
        // shared with the driver protocol, never the wire crate itself.
        "tmt-core" => &[
            "tmt-host-grammar",
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
        // Adapters run host drivers through the protocol crate (#570).
        "tmt-adapters" => &[
            // Safe macOS process inspection and UTC formatting of legacy ps tokens.
            "tmt-sys",
            "time",
            // Provider-local synchronous WebSocket framing; no core/TLS/async use.
            "tungstenite",
            "tmt-driver-protocol",
            "serde",
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
            "tmt-office-pairing",
            "tmt-office-service",
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
            "tmt-cli-style",
            "tmt-core",
            "tmt-adapters",
            "clap",
            "clap_complete",
            "serde_json",
        ],
        "tmt-command-output" => &["tmt-core", "tmt-adapters", "tmt-cli-style", "serde_json"],
        // The shared CLI style is a leaf: it may depend on no TMT crate, so any
        // CLI, core or extension, can render through it. ratatui is optional
        // (the `ratatui` feature) so full-screen views get the same theme
        // styles while core links none of it.
        "tmt-cli-style" => &[
            "anstream",
            "anstyle",
            "clap",
            "comfy-table",
            "ratatui",
            "shlex",
            "unicode-width",
        ],
        // The driver protocol carries no TMT behavior: a community driver
        // builds against it alone, so its only workspace crate is the grammar.
        "tmt-driver-protocol" => &["serde", "serde_json", "tmt-host-grammar"],
        // The Herdr driver is built like a community driver: the protocol
        // crate alone, with tmt-invoke as its bounded process owner. It never
        // reaches core or the adapters.
        "tmt-driver-herdr" => &["semver", "serde_json", "tmt-driver-protocol", "tmt-invoke"],
        // A host's pane-ID and target syntax, defined once for core and the
        // driver protocol; it depends on nothing.
        "tmt-host-grammar" => &[],
        "tmt-sys" => &["libc"],
        // The command crate also owns the companion invocation boundary and the
        // Office release verifier it hands to native installation.
        "tmt-office-command" => &[
            "tmt-core",
            "tmt-office-model",
            "tmt-adapters",
            "tmt-office-service",
            "tmt-command-output",
            "clap",
            "semver",
            "serde",
            "serde_json",
            "uuid",
        ],
        // Office-owned storage reaches core only through the public config and
        // file-lock owners; it reads the core database directly only to migrate.
        // `nix` measures free space before the switch backup.
        "tmt-office-storage" => &[
            "tmt-adapters",
            "tmt-office-pairing",
            "tmt-office-service",
            "tmt-core",
            "tmt-office-model",
            "rusqlite",
            "base64",
            "nix",
            "serde",
            "sha2",
            "serde_json",
            "uuid",
        ],
        // The loopback service lifecycle and receipt, shared by the commands,
        // Office storage and the companion; no credential store.
        "tmt-office-service" => &[
            "tmt-adapters",
            "tmt-office-model",
            "nix",
            "serde",
            "serde_json",
            "uuid",
        ],
        // Pairing owns the OS credential vault and the paired-service HTTP
        // transport; no core crate depends on it.
        "tmt-office-pairing" => &[
            "tmt-adapters",
            "tmt-core",
            "tmt-office-model",
            "base64",
            "getrandom",
            "keyring-core",
            "zbus-secret-service-keyring-store",
            "apple-native-keyring-store",
            "serde",
            "serde_json",
            "ureq",
            "url",
            "uuid",
        ],
        // Squad is a public-interface consumer: it reaches TMT only through
        // commands and `tmt api`. Reviewed neutral leaves carry no core behavior.
        "tmt-squad" => &[
            "tmt-cli-style",
            // Same neutral bounded process owner used by Remote and Colab.
            "tmt-invoke",
            "tmt-tui",
            "clap",
            "serde_json",
            "toml_edit",
            "subprocess",
            "ratatui",
            "unicode-width",
            "signal-hook",
            "pulldown-cmark",
            // Disposable observed-age cache: content fingerprints and
            // nonblocking Unix advisory locking, no TMT behavior.
            "sha2",
            "nix",
        ],
        "tmt-invoke" => &["subprocess", "nix"],
        // Taffy owns admitted flex/grid geometry; scalar measurement stays injected.
        "tmt-tui" => &["roxmltree", "serde_json", "tmt-cli-style", "taffy"],
        "tmt-colab" => &[
            "serde",
            "yrs",
            "base64",
            "ed25519-dalek",
            "getrandom",
            "nix",
            "rusqlite",
            "sha2",
            "clap",
            "httparse",
            "serde_json",
            "signal-hook",
            "tmt-cli-style",
            "tmt-invoke",
        ],
        // Colab model owns pure codecs and fixed crypto, not core or extension behavior.
        "tmt-colab-model" => &[
            "hpke",
            "x25519-dalek",
            "aes-gcm",
            "base64",
            "ed25519-dalek",
            "getrandom",
            "hmac",
            "serde",
            "serde_json",
            "sha2",
        ],
        "tmt-remote" => &[
            // Strict unpadded base64url for contract binary fields (#1039).
            "base64",
            "ed25519-dalek",
            "hmac",
            "sha2",
            "tmt-cli-style",
            "tmt-invoke",
            "clap",
            "serde_json",
            "getrandom",
            "httparse",
            "signal-hook",
            // Relocated colab door (#1039): poll-bounded accept and reply drain.
            "nix",
        ],
        _ => return vec![format!("unreviewed workspace package {name}")],
    };
    package["dependencies"]
        .as_array()
        .expect("Cargo dependencies")
        .iter()
        .filter_map(|d| {
            let dependency = d["name"].as_str().expect("Cargo dependency name");
            if d["kind"] == "dev" && !["tmt-invoke", "tmt-tui"].contains(&name) {
                let target = d["target"].as_str();
                let entry = format!("({name:?}, {dependency:?}, {target:?}),");
                let ledger = "DEV_DEPENDENCIES in rust/crates/tmt-cli/tests/architecture/policy.rs";
                let reviewed = DEV_DEPENDENCIES.contains(&(name, dependency, target));
                if reviewed && d["rename"].is_null() {
                    return None;
                }
                let action = if reviewed {
                    format!("remove rename from the {name} Cargo.toml; {ledger} already reviews {entry}")
                } else {
                    format!("remove any rename and, after review, add {entry} // <review reason> to {ledger}")
                };
                return Some(format!(
                    "{name}: unreviewed dev dependency {dependency} (target={}, rename={}); {action}",
                    d["target"], d["rename"]
                ));
            }
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

// Office extension crates consume the Office model; core crates never do.
fn office_consumer(source: &Source) -> bool {
    matches!(
        source.package.as_str(),
        "tmt-office-command" | "tmt-office-storage" | "tmt-office-pairing" | "tmt-office-service"
    )
}

/// Office modules still declared in core crates while #355 extracts them. The
/// list only shrinks: a core crate may not add an `office_*` module.
const CORE_OFFICE_MODULES: &[&str] = &[
    // tmt-cli: the `tmt office` facade and the Office release verifier it
    // lends core's installers (`tmt extension`, `__native-install`), removed
    // with PR B.
    "office_facade",
];

/// Core crates never own Office modules beyond the retained list above.
fn core_office_module(source: &Source) -> Option<String> {
    if !["tmt-core", "tmt-adapters", "tmt-cli", "tmt-command-output"]
        .contains(&source.package.as_str())
    {
        return None;
    }
    let module = source.file.split(['/', '.']).next()?;
    (module.starts_with("office_") && !CORE_OFFICE_MODULES.contains(&module)).then(|| {
        format!(
            "{}/{}: core crates cannot declare Office module {module}",
            source.package, source.file
        )
    })
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
    violations.extend(sources.iter().filter_map(core_office_module));
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
                // The approved drivers' host syntax, written once at start (#570).
                let host_registry = source.package == "tmt-core"
                    && source.file == "host.rs"
                    && module == "sync"
                    && path.get(2).is_some_and(|p| p == "OnceLock");
                if (root == "std" && !pure_std.contains(&module) && !model_value && !host_registry)
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
                && !["dispatch", "limits", "content_digest", "repository_id"].contains(&module)
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
            if source.package == "tmt-colab-model"
                && ((root.starts_with("tmt_") && root != "tmt_colab_model")
                    || (["std", "core"].contains(&root)
                        && ["fs", "io", "net", "process", "env", "thread"].contains(&module)))
            {
                violations.push(format!(
                    "{location}: colab model cannot acquire runtime authority via {}",
                    path.join("::")
                ));
            }
            if root == "tmt_colab_model" && source.package != "tmt-colab-model" {
                violations.push(format!(
                    "{location}: unreviewed colab model consumer {}",
                    source.package
                ));
            }
            if source.package == "tmt-colab" && root == "yrs" && source.file != "decoder/child.rs" {
                violations.push(format!(
                    "{location}: foreign Yjs decoding belongs only in the decoder child"
                ));
            }
            // Public-interface extensions name their own library and approved leaves only.
            if ["tmt-squad", "tmt-remote", "tmt-colab"].contains(&source.package.as_str())
                && root.starts_with("tmt_")
                && root != "tmt_cli_style"
                && root != "tmt_invoke"
                && !(source.package == "tmt-squad" && root == "tmt_tui")
                && root != source.package.replace('-', "_")
            {
                violations.push(format!(
                    "{location}: {} reaches TMT only through public commands, not {}",
                    source.package.strip_prefix("tmt-").unwrap(),
                    path.join("::")
                ));
            }
            if source.package == "tmt-invoke" && root.starts_with("tmt_") && root != "tmt_invoke" {
                violations.push(format!(
                    "{location}: invoke leaf cannot reach {}",
                    path.join("::")
                ));
            }
            if source.package == "tmt-tui"
                && root.starts_with("tmt_")
                && !["tmt_tui", "tmt_cli_style"].contains(&root)
            {
                violations.push(format!(
                    "{location}: TUI leaf cannot reach {}",
                    path.join("::")
                ));
            }
            if root == "tmt_tui" && !["tmt-tui", "tmt-squad"].contains(&source.package.as_str()) {
                violations.push(format!(
                    "{location}: unreviewed TUI consumer {}",
                    source.package
                ));
            }
            // Terminal hosts are reached through the host port (#486); only it
            // and the tmux module itself name tmux.
            let names_tmux = (root == "tmt_adapters" && module == "tmux")
                || (source.package == "tmt-adapters" && root == "crate" && module == "tmux");
            let host_owner = source.package == "tmt-adapters"
                && (source.file == "host.rs" || source.file.starts_with("tmux/"));
            if names_tmux && !host_owner {
                violations.push(format!(
                    "{location}: reach the terminal host through tmt_adapters::host, not {}",
                    path.join("::")
                ));
            }
            if ["tmt_squad", "tmt_remote"].contains(&root)
                && source.package.replace('-', "_") != root
            {
                violations.push(format!(
                    "{location}: no package may depend on the {} extension: {}",
                    root.strip_prefix("tmt_").unwrap(),
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
