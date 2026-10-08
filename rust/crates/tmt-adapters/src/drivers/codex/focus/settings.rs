//! Narrow read-only admission of hook sources, not a provider config resolver.
//! Unsupported layers use the launcher's original-command fallback.
use super::*;
use crate::{bounded_file, skill_installation::ProviderEnvironment};
use std::{ffi::OsString, fs, path::PathBuf};

fn read(path: &Path) -> io::Result<Option<String>> {
    match bounded_file::read_no_follow(path, setup::SETTINGS_LIMIT) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| unavailable()),
        Err(bounded_file::FileReadError::Io(e))
            if e.kind() == io::ErrorKind::NotFound && fs::symlink_metadata(path).is_err() =>
        {
            Ok(None)
        }
        Err(_) => Err(unavailable()),
    }
}

fn item(node: &Item) -> io::Result<Value> {
    if let Some(table) = node.as_table_like() {
        return table
            .iter()
            .map(|(k, v)| Ok((k.to_owned(), item_value(v)?)))
            .collect::<io::Result<serde_json::Map<_, _>>>()
            .map(Value::Object);
    }
    if let Some(tables) = node.as_array_of_tables() {
        return tables
            .iter()
            .map(|table| item(&Item::Table(table.clone())))
            .collect::<io::Result<Vec<_>>>()
            .map(Value::Array);
    }
    item_value(node)
}
fn item_value(value: &Item) -> io::Result<Value> {
    if let Some(v) = value.as_value() {
        return atom(v);
    }
    if value.as_table_like().is_some() || value.as_array_of_tables().is_some() {
        return item(value);
    }
    Err(unavailable())
}
fn atom(v: &toml_edit::Value) -> io::Result<Value> {
    if let Some(s) = v.as_str() {
        return Ok(json!(s));
    }
    if let Some(b) = v.as_bool() {
        return Ok(json!(b));
    }
    if let Some(n) = v.as_integer() {
        return Ok(json!(n));
    }
    if let Some(n) = v.as_float() {
        return serde_json::Number::from_f64(n)
            .map(Value::Number)
            .ok_or_else(unavailable);
    }
    if let Some(a) = v.as_array() {
        return a
            .iter()
            .map(atom)
            .collect::<io::Result<Vec<_>>>()
            .map(Value::Array);
    }
    if let Some(t) = v.as_inline_table() {
        return t
            .iter()
            .map(|(k, v)| Ok((k.to_owned(), atom(v)?)))
            .collect::<io::Result<serde_json::Map<_, _>>>()
            .map(Value::Object);
    }
    Err(unavailable())
}

fn inspect(document: &DocumentMut) -> io::Result<Value> {
    for key in ["profile", "plugins"] {
        // These can activate additional hook sources. Do not approximate their
        // effective source selection or make a trust decision for the provider.
        if document.get(key).is_some() {
            return Err(unavailable());
        }
    }
    for key in ["hooks", "codex_hooks"] {
        if let Some(v) = document.get("features").and_then(|i| i.get(key))
            && v.as_bool() != Some(true)
        {
            return Err(unavailable());
        }
    }
    if let Some(v) = document.get("allow_managed_hooks_only")
        && v.as_bool() != Some(false)
    {
        return Err(unavailable());
    }
    let hooks = document
        .get("hooks")
        .map(item)
        .transpose()?
        .unwrap_or_else(|| json!({}));
    let map = hooks.as_object().ok_or_else(unavailable)?;
    if map
        .keys()
        .any(|key| key == "managed_dir" || key == "windows_managed_dir")
    {
        return Err(unavailable());
    }
    Ok(json!({"hooks":hooks}))
}

pub(super) fn sources(
    environment: &ProviderEnvironment,
    cwd: &Path,
    system: &Path,
) -> io::Result<Vec<String>> {
    let home = super::super::codex_home(environment);
    let mut documents = Vec::new();
    for root in [system, home.as_path()] {
        if let Some(text) = read(&root.join("hooks.json"))? {
            // Exact JSON ownership validation is shared with setup.
            documents.push(text);
        }
        for name in ["config.toml", "requirements.toml"] {
            if let Some(text) = read(&root.join(name))? {
                documents.push(
                    inspect(&text.parse::<DocumentMut>().map_err(|_| unavailable())?)?.to_string(),
                );
            }
        }
    }
    for (depth, root) in cwd.ancestors().enumerate() {
        if depth >= 128 {
            return Err(unavailable());
        }
        let root = root.join(".codex");
        if root == home {
            continue;
        }
        if read(&root.join("hooks.json"))?.is_some() {
            return Err(unavailable());
        }
        if let Some(text) = read(&root.join("config.toml"))? {
            let doc = text.parse::<DocumentMut>().map_err(|_| unavailable())?;
            let projected = inspect(&doc)?;
            if projected["hooks"]
                .as_object()
                .is_none_or(|map| !map.is_empty())
            {
                return Err(unavailable());
            }
        }
    }
    Ok(documents)
}

pub(super) fn launch_settings(plan: &LaunchHooks<'_>) -> io::Result<(Value, PathBuf)> {
    let mut session = DocumentMut::new();
    let mut cwd = plan.environment.resolve(Path::new("."));
    let boundary = plan
        .command
        .args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(plan.command.args.len());
    let mut args = plan.command.args[..boundary].iter();
    let mut hook_keys = std::collections::HashSet::new();
    while let Some(arg) = args.next() {
        let arg = arg.to_str().ok_or_else(unavailable)?;
        let (key, inline) = if arg.starts_with("-c") && !arg.starts_with("--") && arg.len() > 2 {
            ("-c", Some(arg[2..].strip_prefix('=').unwrap_or(&arg[2..])))
        } else if arg.starts_with("-C") && arg.len() > 2 {
            ("-C", Some(arg[2..].strip_prefix('=').unwrap_or(&arg[2..])))
        } else if arg.starts_with("-p") && !arg.starts_with("--") && arg.len() > 2 {
            return Err(unavailable());
        } else {
            arg.split_once('=')
                .map_or((arg, None), |(k, v)| (k, Some(v)))
        };
        if matches!(
            key,
            "--ignore-user-config"
                | "--ignore-config"
                | "--profile"
                | "-p"
                | "--dangerously-bypass-hook-trust"
        ) {
            return Err(unavailable());
        }
        if !matches!(
            key,
            "-c" | "--config"
                | "--enable"
                | "--disable"
                | "-C"
                | "--cd"
                | "-m"
                | "--model"
                | "-a"
                | "--ask-for-approval"
                | "-s"
                | "--sandbox"
        ) {
            continue;
        }
        let value = inline
            .map(OsString::from)
            .or_else(|| args.next().cloned())
            .ok_or_else(unavailable)?;
        let value = value.to_str().ok_or_else(unavailable)?;
        if matches!(key, "-C" | "--cd") {
            cwd = plan.environment.resolve(Path::new(value));
        }
        if matches!(key, "--disable") && matches!(value, "hooks" | "codex_hooks") {
            return Err(unavailable());
        }
        if !matches!(key, "-c" | "--config") {
            continue;
        }
        let doc = value.parse::<DocumentMut>().map_err(|_| unavailable())?;
        let projected = inspect(&doc)?;
        if let Some(hooks) = doc.get("hooks") {
            for name in projected["hooks"]
                .as_object()
                .ok_or_else(unavailable)?
                .keys()
            {
                if !hook_keys.insert(name.to_owned()) {
                    return Err(unavailable());
                }
            }
            if session.get("hooks").is_none() {
                session["hooks"] = Item::Table(toml_edit::Table::new());
            }
            for (k, v) in hooks.as_table_like().ok_or_else(unavailable)?.iter() {
                session["hooks"][k] = v.clone();
            }
        }
    }
    Ok((inspect(&session)?, cwd))
}

pub(super) fn toml(value: &Value) -> io::Result<String> {
    match value {
        Value::String(s) => Ok(toml_edit::Value::from(s.as_str()).to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Array(a) => Ok(format!(
            "[{}]",
            a.iter()
                .map(toml)
                .collect::<io::Result<Vec<_>>>()?
                .join(",")
        )),
        Value::Object(o) => Ok(format!(
            "{{{}}}",
            o.iter()
                .map(|(k, v)| Ok(format!(
                    "{}={}",
                    toml_edit::Value::from(k.as_str()),
                    toml(v)?
                )))
                .collect::<io::Result<Vec<_>>>()?
                .join(",")
        )),
        Value::Null => Err(unavailable()),
    }
}
