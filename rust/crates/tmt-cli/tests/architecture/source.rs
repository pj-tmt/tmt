//! Test-only Rust syntax collection. No source-text pattern matching or macro
//! expansion: unsupported module remapping fails rather than hiding files.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use syn::{
    Attribute, Item, Meta, Token,
    parse::Parser,
    punctuated::Punctuated,
    visit::{self, Visit},
};

pub struct Source {
    pub package: String,
    pub file: String,
    pub syntax: syn::File,
}

pub fn attributes(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(i) => &i.attrs,
        Item::Enum(i) => &i.attrs,
        Item::ExternCrate(i) => &i.attrs,
        Item::Fn(i) => &i.attrs,
        Item::ForeignMod(i) => &i.attrs,
        Item::Impl(i) => &i.attrs,
        Item::Macro(i) => &i.attrs,
        Item::Mod(i) => &i.attrs,
        Item::Static(i) => &i.attrs,
        Item::Struct(i) => &i.attrs,
        Item::Trait(i) => &i.attrs,
        Item::TraitAlias(i) => &i.attrs,
        Item::Type(i) => &i.attrs,
        Item::Union(i) => &i.attrs,
        Item::Use(i) => &i.attrs,
        _ => &[],
    }
}

// Unknown platform/feature flags stay unknown. Only a provably false branch
// with test=false may be skipped; cfg(any(test, unix)) is still production.
fn cfg_value(meta: &Meta) -> Option<bool> {
    match meta {
        Meta::Path(path) if path.is_ident("test") => Some(false),
        Meta::List(list) => {
            let args = Punctuated::<Meta, Token![,]>::parse_terminated
                .parse2(list.tokens.clone())
                .ok()?;
            let values: Vec<_> = args.iter().map(cfg_value).collect();
            if list.path.is_ident("all") {
                if values.contains(&Some(false)) {
                    Some(false)
                } else if values.iter().all(|v| *v == Some(true)) {
                    Some(true)
                } else {
                    None
                }
            } else if list.path.is_ident("any") {
                if values.contains(&Some(true)) {
                    Some(true)
                } else if values.iter().all(|v| *v == Some(false)) {
                    Some(false)
                } else {
                    None
                }
            } else if list.path.is_ident("not") && values.len() == 1 {
                values[0].map(|v| !v)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn production(item: &Item) -> bool {
    production_attributes(attributes(item))
}

fn production_attributes(attributes: &[Attribute]) -> bool {
    !attributes.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr.parse_args::<Meta>().ok().and_then(|m| cfg_value(&m)) == Some(false)
    })
}

pub fn production_impl(item: &syn::ImplItem) -> bool {
    production_attributes(match item {
        syn::ImplItem::Const(i) => &i.attrs,
        syn::ImplItem::Fn(i) => &i.attrs,
        syn::ImplItem::Type(i) => &i.attrs,
        syn::ImplItem::Macro(i) => &i.attrs,
        _ => &[],
    })
}

pub fn production_trait(item: &syn::TraitItem) -> bool {
    production_attributes(match item {
        syn::TraitItem::Const(i) => &i.attrs,
        syn::TraitItem::Fn(i) => &i.attrs,
        syn::TraitItem::Type(i) => &i.attrs,
        syn::TraitItem::Macro(i) => &i.attrs,
        _ => &[],
    })
}

struct Modules {
    directory: PathBuf,
    children: Vec<PathBuf>,
    errors: Vec<String>,
    allow_generated_asset_include: bool,
}

impl<'ast> Visit<'ast> for Modules {
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
        if production(item) {
            if matches!(item, Item::Verbatim(_)) {
                self.errors.push("unsupported verbatim Rust item".into());
                return;
            }
            visit::visit_item(self, item);
        }
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        if node.path.is_ident("include") && !self.allow_generated_asset_include {
            self.errors
                .push("source include! requires explicit collector support".into());
        }
        visit::visit_macro(self, node);
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if item
            .attrs
            .iter()
            .any(|a| a.path().is_ident("path") || a.path().is_ident("cfg_attr"))
        {
            self.errors
                .push(format!("unsupported module remapping on {}", item.ident));
            return;
        }
        let name = item.ident.to_string();
        if let Some((_, items)) = &item.content {
            let previous = self.directory.clone();
            self.directory.push(&name);
            for item in items {
                self.visit_item(item);
            }
            self.directory = previous;
        } else {
            let candidates = [
                self.directory.join(format!("{name}.rs")),
                self.directory.join(&name).join("mod.rs"),
            ];
            let found: Vec<_> = candidates
                .into_iter()
                .filter(|path| path.is_file())
                .collect();
            if found.len() == 1 {
                self.children.push(found[0].clone());
            } else {
                self.errors.push(format!(
                    "module {name} needs exactly one source file under {}",
                    self.directory.display()
                ));
            }
        }
    }
}

pub fn collect(package: &str, root: &Path) -> Result<Vec<Source>, String> {
    let directory = root
        .parent()
        .ok_or("source root has no directory")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let mut sources = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    let mut seen = BTreeSet::new();
    while let Some(path) = pending.pop() {
        let path = path
            .canonicalize()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if !path.starts_with(&directory) || !seen.insert(path.clone()) {
            return Err(format!("duplicate or escaped module: {}", path.display()));
        }
        let file = path
            .strip_prefix(&directory)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(&path).map_err(|e| format!("{file}: {e}"))?;
        let syntax = syn::parse_file(&text).map_err(|e| format!("{file}: {e}"))?;
        let mut module_dir = path
            .parent()
            .ok_or("module has no directory")?
            .to_path_buf();
        if !matches!(
            path.file_name().and_then(|v| v.to_str()),
            Some("lib.rs" | "main.rs" | "mod.rs")
        ) {
            module_dir.push(path.file_stem().ok_or("module has no stem")?);
        }
        let mut modules = Modules {
            directory: module_dir,
            children: Vec::new(),
            errors: Vec::new(),
            // tmt-office/build.rs and tmt-colab/build.rs are the bounded
            // inventory owners for these generated const asset tables.
            // Production dependencies remain visible in their source modules.
            allow_generated_asset_include: matches!(
                (package, file.as_str()),
                ("tmt-office", "local_assets.rs") | ("tmt-colab", "assets.rs" | "hosting.rs")
            ),
        };
        modules.visit_file(&syntax);
        if !modules.errors.is_empty() {
            return Err(format!("{file}: {}", modules.errors.join("; ")));
        }
        pending.extend(modules.children);
        sources.push(Source {
            package: package.into(),
            file,
            syntax,
        });
    }
    sources.sort_by(|a, b| a.file.cmp(&b.file));
    Ok(sources)
}
