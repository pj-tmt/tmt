//! Project-only declarations are admitted by CI, without executing captured source.
//! This all-file syntax guard deliberately performs no cfg or reachability analysis.

use proc_macro2::{TokenStream, TokenTree};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};
use syn::{
    Expr, Lit, Meta,
    visit::{self, Visit},
};
use tmt_adapters::process::{CommandRequest, CommandRunner, UnixCommandRunner};
use toml_edit::DocumentMut;

type GuardResult<T> = Result<T, String>;

fn text<'a>(value: &'a Value, key: &str) -> GuardResult<&'a str> {
    value[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("missing declaration {key}"))
}

fn entries<'a>(value: &'a Value, key: &str) -> GuardResult<&'a [Value]> {
    if value[key].is_null() {
        return Ok(&[]);
    }
    value[key]
        .as_array()
        .filter(|v| !v.is_empty())
        .map(Vec::as_slice)
        .ok_or_else(|| format!("invalid declaration {key}"))
}

fn relative(root: &Path, path: &Path) -> GuardResult<String> {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .map_err(|_| format!("input escapes repository: {}", path.display()))
}

fn declared_path(root: &Path, value: &Value, key: &str) -> GuardResult<PathBuf> {
    let name = text(value, key)?;
    if name
        .split('/')
        .any(|s| s.is_empty() || s == "." || s == "..")
        || name.contains(['\\', '*', '?', '[', ']', ':'])
        || name.chars().any(char::is_control)
    {
        return Err(format!("invalid literal {key}: {name}"));
    }
    Ok(root.join(name))
}

fn normalized(root: &Path, base: &Path, name: &str) -> GuardResult<PathBuf> {
    let mut result = PathBuf::new();
    for component in base.join(name).components() {
        match component {
            Component::ParentDir => {
                if !result.pop() {
                    return Err("escaped input".into());
                }
            }
            Component::CurDir => {}
            _ => result.push(component.as_os_str()),
        }
    }
    relative(root, &result)?;
    Ok(result)
}

fn overlap(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

struct Declaration {
    root: PathBuf,
    exceptions: Vec<PathBuf>,
    used: BTreeSet<PathBuf>,
}

fn protect(declarations: &[Declaration], input: &Path) -> GuardResult<()> {
    if let Some(declaration) = declarations.iter().find(|d| overlap(&d.root, input)) {
        return Err(format!(
            "shipped input {} overlaps {}",
            input.display(),
            declaration.root.display()
        ));
    }
    Ok(())
}

fn rust_files(
    directory: &Path,
    declarations: &[Declaration],
    files: &mut Vec<PathBuf>,
) -> GuardResult<()> {
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err(format!("ambiguous symlink input {}", path.display()));
        }
        if declarations.iter().any(|d| path.starts_with(&d.root)) {
            continue;
        }
        if kind.is_dir() {
            rust_files(&path, declarations, files)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

#[derive(Debug)]
struct Reference {
    kind: String,
    target: Option<String>,
    expression: String,
}

#[derive(Default)]
struct References {
    values: Vec<Reference>,
}

fn include_name(name: &str) -> bool {
    matches!(name, "include" | "include_str" | "include_bytes")
}

impl References {
    fn include(&mut self, kind: &str, tokens: TokenStream) {
        let target = syn::parse2::<syn::LitStr>(tokens.clone())
            .ok()
            .map(|s| s.value());
        self.values.push(Reference {
            kind: kind.into(),
            target,
            expression: tokens.to_string(),
        });
    }

    fn path_value(&mut self, value: &Expr) {
        let target = match value {
            Expr::Lit(lit) => match &lit.lit {
                Lit::Str(s) => Some(s.value()),
                _ => None,
            },
            _ => None,
        };
        self.values.push(Reference {
            kind: "path".into(),
            target,
            expression: String::new(),
        });
    }

    fn attribute(&mut self, meta: &Meta) {
        if meta.path().is_ident("path") {
            match meta {
                Meta::NameValue(meta) => self.path_value(&meta.value),
                _ => self.values.push(Reference {
                    kind: "path".into(),
                    target: None,
                    expression: String::new(),
                }),
            }
        } else if meta.path().is_ident("cfg_attr") {
            // Refuse conditional remapping, including attributes hidden in macro groups.
            if let Meta::List(meta) = meta {
                let tokens = meta.tokens.to_string();
                if tokens.split_whitespace().any(|part| part == "path") {
                    self.values.push(Reference {
                        kind: "cfg_attr path".into(),
                        target: None,
                        expression: tokens,
                    });
                }
            }
        }
    }

    // Macro bodies are opaque to syn's AST visitor. Inspect groups, never literal contents.
    fn tokens(&mut self, stream: TokenStream) {
        let tokens: Vec<_> = stream.into_iter().collect();
        let mut index = 0;
        while index < tokens.len() {
            if let (
                Some(TokenTree::Ident(name)),
                Some(TokenTree::Punct(bang)),
                Some(TokenTree::Group(args)),
            ) = (
                tokens.get(index),
                tokens.get(index + 1),
                tokens.get(index + 2),
            ) && include_name(&name.to_string())
                && bang.as_char() == '!'
            {
                self.include(&name.to_string(), args.stream());
                index += 3;
                continue;
            }
            if let (Some(TokenTree::Punct(hash)), Some(TokenTree::Group(group))) =
                (tokens.get(index), tokens.get(index + 1))
                && hash.as_char() == '#'
            {
                if let Ok(meta) = syn::parse2::<Meta>(group.stream()) {
                    self.attribute(&meta);
                }
                index += 2;
                continue;
            }
            if let TokenTree::Group(group) = &tokens[index] {
                self.tokens(group.stream());
            }
            index += 1;
        }
    }
}

impl<'ast> Visit<'ast> for References {
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        let name = node.path.segments.last().unwrap().ident.to_string();
        if include_name(&name) {
            self.include(&name, node.tokens.clone());
        } else {
            self.tokens(node.tokens.clone());
        }
    }

    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        self.attribute(&attr.meta);
        visit::visit_attribute(self, attr);
    }
}

fn exception_parent(file: &Path, files: &[PathBuf]) -> GuardResult<()> {
    let directory = file.parent().ok_or("exception has no directory")?;
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("exception has no name")?;
    let parent = directory.with_extension("rs");
    let candidates = [parent, directory.join("mod.rs")];
    let parents: Vec<_> = candidates.iter().filter(|p| files.contains(p)).collect();
    if parents.len() != 1 {
        return Err(format!(
            "missing or ambiguous test-only parent for {}",
            file.display()
        ));
    }
    let source = fs::read_to_string(parents[0]).map_err(|e| e.to_string())?;
    let syntax = syn::parse_file(&source).map_err(|e| e.to_string())?;
    let modules: Vec<_> = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Mod(module) if module.ident == stem => Some(module),
            _ => None,
        })
        .collect();
    if modules.len() != 1 || modules[0].content.is_some()
        || modules[0].attrs.iter().any(|a| a.path().is_ident("path") || a.path().is_ident("cfg_attr"))
        || !modules[0].attrs.iter().any(|a| matches!(&a.meta, Meta::List(m) if m.path.is_ident("cfg") && m.tokens.to_string() == "test")) {
        return Err("test-only parent requires immediate #[cfg(test)] on the actual module".into());
    }
    let lines: Vec<_> = source.lines().map(str::trim).collect();
    let declarations: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            [
                format!("mod {stem};"),
                format!("pub mod {stem};"),
                format!("pub(crate) mod {stem};"),
            ]
            .contains(&line.to_string())
        })
        .map(|(index, _)| index)
        .collect();
    if declarations.len() != 1
        || declarations[0] == 0
        || lines[declarations[0] - 1] != "#[cfg(test)]"
    {
        return Err(format!(
            "test-only parent requires immediate #[cfg(test)] for {}",
            file.display()
        ));
    }
    Ok(())
}

fn protect_includes(
    root: &Path,
    base: &Path,
    includes: &toml_edit::Array,
    declarations: &[Declaration],
) -> GuardResult<()> {
    for input in includes {
        let input = input.as_str().ok_or("nonliteral package include")?;
        if input.contains(['*', '?', '[', ']']) {
            return Err("ambiguous package include glob".into());
        }
        protect(declarations, &normalized(root, base, input)?)?;
    }
    Ok(())
}

fn generator(
    root: &Path,
    entry: &Value,
    crate_dir: &Path,
    declarations: &[Declaration],
) -> GuardResult<(PathBuf, String)> {
    let site = declared_path(root, entry, "includeSite")?;
    let generated = declared_path(root, entry, "generator")?;
    let build = declared_path(root, entry, "buildScript")?;
    let directory = declared_path(root, entry, "inputDirectory")?;
    let package = declared_path(root, entry, "packageRoot")?;
    let release = declared_path(root, entry, "releaseScript")?;
    let variable = text(entry, "variable")?;
    text(entry, "reason")?;
    let hosted = variable == "TMT_COLAB_HOSTING_DIR";
    let (include_site, generator_file, input_directory, output_file) = if hosted {
        (
            "src/hosting.rs",
            "build/hosting.rs",
            "dist-hosted",
            "/colab_hosting.rs",
        )
    } else {
        (
            "src/assets.rs",
            "build/assets.rs",
            "dist",
            "/colab_assets.rs",
        )
    };
    if site != crate_dir.join(include_site)
        || generated != crate_dir.join(generator_file)
        || build != crate_dir.join("build.rs")
        || (!hosted && variable != "TMT_COLAB_APP_DIR")
        || directory != package.join(input_directory)
        || release != root.join("scripts/build-native-artifact.sh")
        || !package.join("package.json").is_file()
    {
        return Err("unsupported canonical generated pipeline".into());
    }
    for input in [&site, &generated, &build, &directory, &package, &release] {
        protect(declarations, input)?;
    }
    if hosted {
        protect(declarations, &crate_dir.join("src/browser_policy.rs"))?;
        protect(
            declarations,
            &root.join("extensions/tmt-remote/rust/tmt-remote/assets/remote-v1.js"),
        )?;
    }
    let expression = text(entry, "expression")?
        .parse::<TokenStream>()
        .map_err(|e| e.to_string())?
        .to_string();
    let expected = format!("concat!(env!(\"OUT_DIR\"), {output_file:?})")
        .parse::<TokenStream>()
        .unwrap()
        .to_string();
    if expression != expected {
        return Err("unsupported generated include expression".into());
    }
    let script = fs::read_to_string(&release).map_err(|e| e.to_string())?;
    let lines: Vec<_> = script.lines().map(str::trim).collect();
    let reset = lines
        .iter()
        .position(|line| *line == "unset TMT_COLAB_HOSTING_DIR");
    let export = lines
        .iter()
        .position(|line| *line == "export TMT_COLAB_APP_DIR");
    let product_block = lines
        .iter()
        .position(|line| line.starts_with("if [ \"$product\" ="));
    if lines
        .iter()
        .filter(|line| **line == "unset TMT_COLAB_HOSTING_DIR")
        .count()
        != 1
        || reset
            .zip(export)
            .is_none_or(|(reset, export)| reset >= export)
        || product_block.is_some_and(|block| reset.is_none_or(|reset| reset >= block))
        || lines.iter().any(|line| {
            line.starts_with("TMT_COLAB_HOSTING_DIR=") || *line == "export TMT_COLAB_HOSTING_DIR"
        })
    {
        return Err("canonical hosted release exclusion drift".into());
    }
    if !hosted {
        let pin = format!("{variable}=\"$repo/{}\"", relative(root, &directory)?);
        let assignments: Vec<_> = script
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with(&format!("{variable}=")))
            .collect();
        if assignments != [pin.as_str()]
            || script
                .lines()
                .map(str::trim)
                .filter(|line| *line == format!("export {variable}"))
                .count()
                != 1
            || script
                .lines()
                .map(str::trim)
                .filter(|line| {
                    *line
                        == "corepack pnpm@10.33.0 --filter @tmt/colab-app --fail-if-no-match build 1>&2"
                })
                .count()
                != 1
        {
            return Err("canonical release directory pin drift".into());
        }
    }
    // One deliberately exact, small forwarding owner. New variables/calls require reviewed proof.
    let expected_build = "#[path = \"src/app_inventory.rs\"]\nmod app_inventory;\n#[path = \"build/assets.rs\"]\nmod assets;\n#[path = \"src/browser_policy.rs\"]\nmod browser_policy;\n#[path = \"build/hosting.rs\"]\nmod hosting;\n\nfn main() {\n    println!(\"cargo:rerun-if-env-changed=TMT_COLAB_APP_DIR\");\n    println!(\"cargo:rerun-if-env-changed=TMT_COLAB_HOSTING_DIR\");\n    let output =\n        std::path::PathBuf::from(std::env::var_os(\"OUT_DIR\").expect(\"Cargo supplies OUT_DIR\"));\n    let directory = std::env::var_os(\"TMT_COLAB_APP_DIR\").map(std::path::PathBuf::from);\n    let inputs = assets::generate(directory.as_deref(), &output)\n        .unwrap_or_else(|error| panic!(\"Colab app build is unavailable: {error}\"));\n    for input in inputs {\n        println!(\"cargo:rerun-if-changed={}\", input.display());\n    }\n    let hosted = std::env::var_os(\"TMT_COLAB_HOSTING_DIR\").map(std::path::PathBuf::from);\n    let sdk = std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\"))\n        .join(\"../../../tmt-remote/rust/tmt-remote/assets/remote-v1.js\");\n    let inputs = hosting::generate(hosted.as_deref(), &sdk, &output)\n        .unwrap_or_else(|error| panic!(\"Colab hosted build is unavailable: {error}\"));\n    for input in inputs {\n        println!(\"cargo:rerun-if-changed={}\", input.display());\n    }\n}\n";
    if fs::read_to_string(&build).map_err(|e| e.to_string())? != expected_build {
        return Err("canonical generator forwarding drift".into());
    }
    // Freeze the one reviewed byte-producing owner, rather than infer its I/O from names.
    // Git hashes a file; it does not execute generator source or require a checkout there.
    let args: Vec<OsString> = ["hash-object", "--no-filters", "--"]
        .into_iter()
        .map(Into::into)
        .chain([generated.into_os_string()])
        .collect();
    let result = UnixCommandRunner
        .execute(CommandRequest {
            program: std::ffi::OsStr::new("git"),
            args: &args,
            input: &[],
            deadline: Instant::now() + Duration::from_secs(10),
            max_output_bytes: 1024,
        })
        .map_err(|e| e.to_string())?;
    let blob = std::str::from_utf8(&result.stdout)
        .map_err(|e| e.to_string())?
        .trim();
    if blob != text(entry, "generatorBlob")? {
        return Err("canonical asset generator drift".into());
    }
    Ok((site, expression))
}

// Resolve only literal include inputs and the manifest-relative concat form.
// Unknown expressions fail closed; do not execute a build script to discover inputs.
fn browser_input(expr: &Expr, manifest: &Path) -> GuardResult<String> {
    match expr {
        Expr::Lit(value) => match &value.lit {
            Lit::Str(value) => Ok(value.value()),
            _ => Err("non-string browser input".into()),
        },
        Expr::Macro(value) if value.mac.path.is_ident("env") => {
            let name = syn::parse2::<syn::LitStr>(value.mac.tokens.clone())
                .map_err(|_| "nonliteral browser input environment")?;
            if name.value() == "CARGO_MANIFEST_DIR" {
                Ok(manifest.to_string_lossy().into())
            } else {
                Err("unproved browser input environment".into())
            }
        }
        Expr::Macro(value) if value.mac.path.is_ident("concat") => {
            use syn::parse::Parser;
            let args = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated
                .parse2(value.mac.tokens.clone())
                .map_err(|e| e.to_string())?;
            args.iter()
                .map(|arg| browser_input(arg, manifest))
                .collect()
        }
        _ => Err("unproved browser input expression".into()),
    }
}

// This is literal input substitution, not macro expansion. Only local, uniquely named
// single-arm wrappers are proved; opaque forwarding and every other form refuse.
struct LiteralWrapper {
    parameters: Vec<String>,
    body: TokenStream,
}

fn literal_wrapper(item: &syn::ItemMacro) -> GuardResult<LiteralWrapper> {
    if !item.attrs.is_empty() {
        return Err("unproved attributed/exported input wrapper".into());
    }
    let tokens: Vec<_> = item.mac.tokens.clone().into_iter().collect();
    let [
        TokenTree::Group(matcher),
        TokenTree::Punct(eq),
        TokenTree::Punct(arrow),
        TokenTree::Group(body),
        rest @ ..,
    ] = tokens.as_slice()
    else {
        return Err("unproved input wrapper arm".into());
    };
    if eq.as_char() != '='
        || arrow.as_char() != '>'
        || !matches!(rest, [] | [TokenTree::Punct(_)])
        || rest
            .first()
            .is_some_and(|t| !matches!(t, TokenTree::Punct(p) if p.as_char() == ';'))
    {
        return Err("unproved input wrapper arms".into());
    }
    let matcher: Vec<_> = matcher.stream().into_iter().collect();
    let mut parameters = Vec::new();
    let mut index = 0;
    while index < matcher.len() {
        let Some(
            [
                TokenTree::Punct(dollar),
                TokenTree::Ident(name),
                TokenTree::Punct(colon),
                TokenTree::Ident(kind),
            ],
        ) = matcher.get(index..index + 4)
        else {
            return Err("unproved input wrapper matcher".into());
        };
        if dollar.as_char() != '$'
            || colon.as_char() != ':'
            || kind != "literal"
            || parameters.contains(&name.to_string())
        {
            return Err("unproved input wrapper binding".into());
        }
        parameters.push(name.to_string());
        index += 4;
        if index < matcher.len() {
            if !matches!(&matcher[index], TokenTree::Punct(p) if p.as_char() == ',') {
                return Err("unproved input wrapper separator".into());
            }
            index += 1;
        }
    }
    if parameters.is_empty() {
        return Err("empty input wrapper matcher".into());
    }
    Ok(LiteralWrapper {
        parameters,
        body: body.stream(),
    })
}

fn substitute_literals(
    stream: TokenStream,
    bindings: &BTreeMap<String, syn::LitStr>,
) -> GuardResult<TokenStream> {
    let tokens: Vec<_> = stream.into_iter().collect();
    let mut output = TokenStream::new();
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            TokenTree::Punct(p) if p.as_char() == '$' => {
                let Some(TokenTree::Ident(name)) = tokens.get(index + 1) else {
                    return Err("unproved input wrapper repetition".into());
                };
                let value = bindings
                    .get(&name.to_string())
                    .ok_or("unproved input wrapper variable")?;
                output.extend(
                    value
                        .token()
                        .to_string()
                        .parse::<TokenStream>()
                        .map_err(|e| e.to_string())?,
                );
                index += 2;
            }
            TokenTree::Group(group) => {
                output.extend([TokenTree::Group(proc_macro2::Group::new(
                    group.delimiter(),
                    substitute_literals(group.stream(), bindings)?,
                ))]);
                index += 1;
            }
            token => {
                output.extend([token.clone()]);
                index += 1;
            }
        }
    }
    Ok(output)
}

fn macro_occurrences(stream: TokenStream, name: &str) -> usize {
    let tokens: Vec<_> = stream.into_iter().collect();
    tokens.iter().enumerate().map(|(index, token)| match token {
        TokenTree::Ident(ident) if ident == name => {
            let call = matches!(tokens.get(index + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!');
            let definition = index >= 2
                && matches!(&tokens[index - 1], TokenTree::Punct(p) if p.as_char() == '!')
                && matches!(&tokens[index - 2], TokenTree::Ident(rule) if rule == "macro_rules");
            usize::from(call || definition)
        }
        TokenTree::Group(group) => macro_occurrences(group.stream(), name),
        _ => 0,
    }).sum()
}

fn imported_wrapper(syntax: &syn::File, name: &str) -> bool {
    struct Imports<'a> {
        name: &'a str,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Imports<'_> {
        fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
            if item.mac.path.is_ident("macro_rules")
                && item.ident.as_ref().is_some_and(|n| n == self.name)
            {
                return;
            }
            visit::visit_item_macro(self, item);
        }
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            fn mentions(stream: TokenStream, name: &str) -> bool {
                let tokens: Vec<_> = stream.into_iter().collect();
                tokens.iter().enumerate().any(|(i, token)| match token {
                    // `value.field` is an ordinary value projection, not a macro name.
                    TokenTree::Ident(ident) if ident == name =>
                        !matches!(tokens.get(i + 1), Some(TokenTree::Punct(dot)) if dot.as_char() == '.'),
                    TokenTree::Group(group) => mentions(group.stream(), name),
                    _ => false,
                })
            }
            // An opaque template that manufactures a use declaration cannot prove
            // wrapper identity through metavariable substitution.
            let tokens = node.tokens.to_string();
            let opaque_import =
                tokens.split_whitespace().any(|t| t == "use") && tokens.contains('$');
            if !node.path.is_ident(self.name)
                && (opaque_import || mentions(node.tokens.clone(), self.name))
            {
                self.found = true;
            }
        }
        fn visit_use_tree(&mut self, tree: &'ast syn::UseTree) {
            match tree {
                syn::UseTree::Name(n) if n.ident == self.name => self.found = true,
                syn::UseTree::Rename(n) if n.ident == self.name || n.rename == self.name => {
                    self.found = true
                }
                syn::UseTree::Path(n) if n.ident == self.name => self.found = true,
                _ => visit::visit_use_tree(self, tree),
            }
        }
    }
    let mut visitor = Imports { name, found: false };
    visitor.visit_file(syntax);
    visitor.found
}

struct WrapperReferences<'a> {
    wrappers: &'a BTreeMap<String, LiteralWrapper>,
    references: References,
    calls: BTreeMap<String, usize>,
    error: Option<String>,
}

impl<'ast> Visit<'ast> for WrapperReferences<'_> {
    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if item.mac.path.is_ident("macro_rules")
            && item
                .ident
                .as_ref()
                .is_some_and(|name| self.wrappers.contains_key(&name.to_string()))
        {
            return;
        }
        visit::visit_item_macro(self, item);
    }
    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        self.references.visit_attribute(attr);
    }
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        let name = node.path.segments.last().unwrap().ident.to_string();
        let Some(wrapper) = self.wrappers.get(&name) else {
            self.references.visit_macro(node);
            return;
        };
        let result = (|| {
            use syn::parse::Parser;
            if !node.path.is_ident(&name) {
                return Err("unproved qualified input wrapper".to_string());
            }
            let args = syn::punctuated::Punctuated::<syn::LitStr, syn::Token![,]>::parse_terminated
                .parse2(node.tokens.clone())
                .map_err(|e| format!("unproved literal wrapper arguments: {e}"))?;
            if args.len() != wrapper.parameters.len() {
                return Err("input wrapper argument count".into());
            }
            let bindings = wrapper.parameters.iter().cloned().zip(args).collect();
            let body = substitute_literals(wrapper.body.clone(), &bindings)?;
            // Reject further macro forwarding. Built-in include/concat/env inputs are
            // still independently scanned and resolved by the existing owner below.
            fn builtins(stream: TokenStream) -> GuardResult<()> {
                let tokens: Vec<_> = stream.into_iter().collect();
                for window in tokens.windows(2) {
                    if let [TokenTree::Ident(name), TokenTree::Punct(bang)] = window
                        && bang.as_char() == '!'
                        && !include_name(&name.to_string())
                        && name != "concat"
                        && name != "env"
                    {
                        return Err("unproved input wrapper forwarding".into());
                    }
                }
                for token in tokens {
                    if let TokenTree::Group(g) = token {
                        builtins(g.stream())?;
                    }
                }
                Ok(())
            }
            builtins(body.clone())?;
            self.references.tokens(body);
            *self.calls.entry(name).or_default() += 1;
            Ok(())
        })();
        if let Err(error) = result {
            self.error = Some(error);
        }
    }
}

fn browser_references(
    file: &Path,
    sources: &[(PathBuf, syn::File, TokenStream)],
) -> GuardResult<References> {
    let (_, syntax, source_tokens) = sources.iter().find(|(path, _, _)| path == file).unwrap();
    let mut wrappers = BTreeMap::new();
    for item in &syntax.items {
        if let syn::Item::Macro(item) = item
            && item.mac.path.is_ident("macro_rules")
        {
            let mut references = References::default();
            references.tokens(item.mac.tokens.clone());
            if references.values.iter().any(|r| r.expression.contains('$')) {
                let name = item
                    .ident
                    .as_ref()
                    .ok_or("unnamed input wrapper")?
                    .to_string();
                if wrappers.insert(name, literal_wrapper(item)?).is_some() {
                    return Err("ambiguous input wrapper definition".into());
                }
            }
        }
    }
    for name in wrappers.keys() {
        let tokens: Vec<_> = source_tokens.clone().into_iter().collect();
        let definition = tokens.windows(3).position(|w| {
            matches!(w,
            [TokenTree::Ident(rule), TokenTree::Punct(bang), TokenTree::Ident(ident)]
            if rule == "macro_rules" && bang.as_char() == '!' && ident == name)
        });
        let prefix = definition.ok_or("unproved local input wrapper definition")?;
        if macro_occurrences(tokens[..prefix].iter().cloned().collect(), name) != 0 {
            return Err("input wrapper use before local definition".into());
        }
    }
    let mut visitor = WrapperReferences {
        wrappers: &wrappers,
        references: References::default(),
        calls: BTreeMap::new(),
        error: None,
    };
    visitor.visit_file(syntax);
    if let Some(error) = visitor.error {
        return Err(format!("{}: {error}", file.display()));
    }
    for name in wrappers.keys() {
        let calls = visitor.calls.get(name).copied().unwrap_or(0);
        // Every macro occurrence must be this definition or a directly visited
        // local call. Ordinary same-named variables live in a different namespace. This refuses alias/export/cross-file/opaque uses and shadowing.
        for (path, syntax, tokens) in sources {
            let count = macro_occurrences(tokens.clone(), name);
            if (path == file && (calls == 0 || count != calls + 1))
                || (path != file && count != 0)
                || imported_wrapper(syntax, name)
            {
                return Err(format!(
                    "{}: unproved input wrapper scope/alias {name}",
                    path.display()
                ));
            }
        }
    }
    Ok(visitor.references)
}

pub fn browser_leaf_inputs(root: &Path, metadata: &Value) -> GuardResult<()> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let leaf = root.join("design/browser-ui");
    for package in metadata["packages"]
        .as_array()
        .ok_or("missing Cargo packages")?
    {
        let manifest = PathBuf::from(text(package, "manifest_path")?);
        let crate_dir = manifest
            .parent()
            .ok_or("missing Core crate directory")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !crate_dir.starts_with(root.join("rust/crates")) {
            continue;
        }
        let mut files = Vec::new();
        rust_files(&crate_dir, &[], &mut files)?;
        let sources = files
            .iter()
            .map(|file| {
                let source = fs::read_to_string(file).map_err(|e| e.to_string())?;
                Ok((
                    file.clone(),
                    syn::parse_file(&source).map_err(|e| format!("{}: {e}", file.display()))?,
                    source.parse::<TokenStream>().map_err(|e| e.to_string())?,
                ))
            })
            .collect::<GuardResult<Vec<_>>>()?;
        for file in files {
            let references = browser_references(&file, &sources)?;
            for reference in references.values {
                let name = match reference.target {
                    Some(name) => name,
                    None if include_name(&reference.kind) => {
                        let expression =
                            syn::parse_str::<Expr>(&reference.expression).map_err(|e| {
                                format!(
                                    "{}: {}({}): {e}",
                                    file.display(),
                                    reference.kind,
                                    reference.expression
                                )
                            })?;
                        browser_input(&expression, &crate_dir).map_err(|e| {
                            format!(
                                "{}: {}({}): {e}",
                                file.display(),
                                reference.kind,
                                reference.expression
                            )
                        })?
                    }
                    None => {
                        return Err(format!(
                            "unproved Core/CLI {} in {}",
                            reference.kind,
                            file.display()
                        ));
                    }
                };
                let target = normalized(&root, file.parent().unwrap(), &name)?;
                // Also reject an indirect filesystem alias into the presentation home.
                if overlap(&target, &leaf)
                    || target.canonicalize().is_ok_and(|p| overlap(&p, &leaf))
                {
                    return Err(format!(
                        "Core/CLI must not embed browser-ui: {} ({})",
                        file.display(),
                        reference.kind
                    ));
                }
            }
        }
    }
    Ok(())
}

pub fn check(root: &Path, metadata: &Value) -> GuardResult<()> {
    let map: Value = serde_json::from_str(
        &fs::read_to_string(root.join(".github/components.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    browser_leaf_inputs(root, metadata)?;
    check_map(root, metadata, &map)
}

fn check_map(root: &Path, metadata: &Value, map: &Value) -> GuardResult<()> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let components = map["components"]
        .as_object()
        .ok_or("missing component map")?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or("missing Cargo packages")?;
    let workspace_dist = fs::read_to_string(root.join("dist-workspace.toml"))
        .map_err(|e| e.to_string())?
        .parse::<DocumentMut>()
        .map_err(|e| e.to_string())?;
    for component in components.values() {
        let roots = entries(component, "neverShippedPaths")?;
        let generated = entries(component, "generatedInputs")?;
        if roots.is_empty() && generated.is_empty() {
            continue;
        }
        if generated.len() > 2
            || (generated.len() == 2
                && (text(&generated[0], "variable")? != "TMT_COLAB_APP_DIR"
                    || text(&generated[1], "variable")? != "TMT_COLAB_HOSTING_DIR"))
        {
            return Err("only the canonical native and hosted generators are permitted".into());
        }
        let package_name = text(component, "package")?;
        let matches: Vec<_> = packages
            .iter()
            .filter(|p| p["name"] == package_name)
            .collect();
        if matches.len() != 1 {
            return Err(format!("missing or ambiguous Cargo package {package_name}"));
        }
        let package = matches[0];
        let manifest = PathBuf::from(text(package, "manifest_path")?);
        let crate_dir = manifest.parent().ok_or("manifest has no directory")?;
        relative(&root, crate_dir)?;
        let mut declarations = Vec::new();
        for entry in roots {
            let path = declared_path(&root, entry, "root")?;
            text(entry, "reason")?;
            let stat = fs::symlink_metadata(&path)
                .map_err(|e| format!("missing declaration root: {e}"))?;
            if stat.file_type().is_symlink()
                || !(stat.is_file() || stat.is_dir())
                || path.canonicalize().map_err(|e| e.to_string())? != path
            {
                return Err("ambiguous declaration root".into());
            }
            let mut exceptions = Vec::new();
            for exception in entries(entry, "testOnlyReferences")? {
                text(exception, "reason")?;
                exceptions.push(declared_path(&root, exception, "file")?);
            }
            declarations.push(Declaration {
                root: path,
                exceptions,
                used: BTreeSet::new(),
            });
        }
        protect(&declarations, &manifest)?;
        protect(&declarations, &root.join("rust/Cargo.lock"))?;
        protect(&declarations, &root.join("dist-workspace.toml"))?;
        for target in package["targets"]
            .as_array()
            .ok_or("missing Cargo targets")?
        {
            let kinds = target["kind"].as_array().ok_or("missing target kinds")?;
            if kinds
                .iter()
                .any(|kind| kind == "bin" || kind == "lib" || kind == "custom-build")
            {
                let path = PathBuf::from(text(target, "src_path")?);
                protect(&declarations, &path)?;
                if !kinds.iter().any(|kind| kind == "custom-build") {
                    protect(
                        &declarations,
                        path.parent().ok_or("target has no source directory")?,
                    )?;
                } else if generated.is_empty()
                    || path != declared_path(&root, &generated[0], "buildScript")?
                {
                    return Err("unproved package build generator".into());
                }
            }
        }
        let document = fs::read_to_string(&manifest)
            .map_err(|e| e.to_string())?
            .parse::<DocumentMut>()
            .map_err(|e| e.to_string())?;
        if let Some(inputs) = document.get("package").and_then(|v| v.get("include")) {
            protect_includes(
                &root,
                crate_dir,
                inputs.as_array().ok_or("invalid package includes")?,
                &declarations,
            )?;
        }
        let dist = package["metadata"]["dist"]["include"].as_array();
        if let Some(inputs) = dist {
            for input in inputs {
                let name = input.as_str().ok_or("nonliteral dist include")?;
                if name.contains(['*', '?', '[', ']']) {
                    return Err("ambiguous dist include glob".into());
                }
                protect(&declarations, &normalized(&root, crate_dir, name)?)?;
            }
        } else {
            let inputs = workspace_dist
                .get("dist")
                .and_then(|v| v.get("include"))
                .and_then(toml_edit::Item::as_array)
                .ok_or("missing workspace dist includes")?;
            protect_includes(&root, &root, inputs, &declarations)?;
        }
        if component["skills"] == true {
            let extension = crate_dir
                .parent()
                .and_then(Path::parent)
                .ok_or("missing product skill owner")?;
            protect(&declarations, &extension.join("skills"))?;
        }
        let allowed = generated
            .iter()
            .map(|entry| generator(&root, entry, crate_dir, &declarations))
            .collect::<GuardResult<Vec<_>>>()?;
        let mut files = Vec::new();
        rust_files(crate_dir, &declarations, &mut files)?;
        files.sort();
        for declaration in &declarations {
            for exception in &declaration.exceptions {
                if !files.contains(exception) {
                    return Err("missing test-only referencing file".into());
                }
                exception_parent(exception, &files)?;
            }
        }
        let mut dynamic = BTreeSet::new();
        for file in &files {
            let source = fs::read_to_string(file).map_err(|e| e.to_string())?;
            let syntax =
                syn::parse_file(&source).map_err(|e| format!("{}: {e}", file.display()))?;
            let mut references = References::default();
            references.visit_file(&syntax);
            for reference in references.values {
                let Some(name) = reference.target else {
                    if reference.kind == "include"
                        && allowed.iter().any(|(site, expression)| {
                            site == file && expression == &reference.expression
                        })
                    {
                        if !dynamic.insert((file.clone(), reference.expression)) {
                            return Err("missing or repeated canonical dynamic include".into());
                        }
                        continue;
                    }
                    return Err(format!(
                        "unproved dynamic {} in {}",
                        reference.kind,
                        file.display()
                    ));
                };
                let target = normalized(&root, file.parent().unwrap(), &name)?;
                let canonical = target
                    .canonicalize()
                    .map_err(|e| format!("missing reference {}: {e}", target.display()))?;
                if canonical != target {
                    return Err("ambiguous reference alias".into());
                }
                for declaration in &mut declarations {
                    // A second source entry into an exempt helper cannot borrow its parent's proof.
                    if declaration.exceptions.contains(&target) {
                        return Err("alternate entry into test-only helper".into());
                    }
                    if overlap(&declaration.root, &target) {
                        if declaration.exceptions.contains(file) {
                            declaration.used.insert(file.clone());
                        } else {
                            return Err(format!(
                                "{} references declared root {}",
                                file.display(),
                                declaration.root.display()
                            ));
                        }
                    }
                }
            }
        }
        if dynamic.len() != allowed.len() {
            return Err("missing or repeated canonical dynamic include".into());
        }
        for declaration in &declarations {
            if declaration.used.len() != declaration.exceptions.len() {
                return Err("stale test-only reference exception".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "never_shipped_cases.rs"]
mod cases;
