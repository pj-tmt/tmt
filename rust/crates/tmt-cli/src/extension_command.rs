//! Compose pure command grammar with the PATH execution adapter.

use crate::grammar::extensions;
use std::{ffi::OsString, io};
use tmt_adapters::extension_command as adapter;
use tmt_core::extension_command::valid_extension_name;

pub fn execute(name: &str, args: &[OsString], help: bool, prefix: &[OsString]) -> io::Result<u8> {
    let search = std::env::var_os("PATH").unwrap_or_default();
    if let Some(executable) = adapter::resolve(name, &search)? {
        if !prefix.is_empty() {
            return crate::failure(
                crate::invocation::OutputMode::default(),
                "USAGE_ERROR",
                "Put options after the extension name: tmt <name> [options].",
            );
        }
        let error = adapter::execute(&executable, args, &std::env::current_exe()?);
        let mut stderr = tmt_cli_style::stream::stderr();
        let terminal = stderr.terminal();
        tmt_cli_style::message::error(
            &mut stderr,
            terminal,
            &format!(
                "Could not execute extension '{}': {error}",
                executable.display()
            ),
            None,
        )?;
        return Ok(1);
    }
    let original: Vec<OsString> = if help {
        vec!["help".into(), name.into()]
    } else {
        prefix
            .iter()
            .cloned()
            .chain(std::iter::once(OsString::from(name)))
            .chain(args.iter().cloned())
            .collect()
    };
    let mut error =
        crate::parser::parse_core(&original).expect_err("external name is not a core command");
    if !help
        && let Some(suggestion) =
            extensions::suggestion(name, adapter::discover(&search)?.into_keys())
    {
        error
            .message
            .push_str(&format!("\nSimilar command: {suggestion}\n"));
    }
    crate::failure(error.mode, error.code, &error.message)
}

/// The `tmt-*` commands on `PATH`, as root help lists them.
#[derive(Debug, Default)]
pub struct Discovered {
    /// Commands named after a reserved core command, which are never run.
    ignored: Vec<(String, std::path::PathBuf)>,
    /// Extension names grouped by the file they resolve to.
    extensions: Vec<(Vec<String>, std::path::PathBuf)>,
}

/// Reads the real `PATH`. Help rendering takes the result as input, so tests
/// can pass a fixed list.
pub fn discover() -> io::Result<Discovered> {
    let reserved = extensions::reserved(&crate::grammar::grammar());
    let mut discovered = Discovered::default();
    let mut extensions = Vec::new();
    for (name, path) in adapter::discover(&std::env::var_os("PATH").unwrap_or_default())? {
        if reserved.contains(&name) {
            discovered.ignored.push((name, path));
        } else if valid_extension_name(&name) {
            let file = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            extensions.push((name, path, file));
        }
    }
    discovered.extensions = group_aliases(extensions);
    Ok(discovered)
}

impl Discovered {
    /// Root help's `Extensions` section; none when nothing was discovered.
    pub fn sections(&self) -> Vec<tmt_cli_style::HelpSection> {
        let home = std::env::home_dir();
        let path = |path: &std::path::Path| {
            tmt_cli_style::table::escape(&tmt_cli_style::value::home_path(path, home.as_deref()))
        };
        let mut entries: Vec<(String, String)> = self
            .extensions
            .iter()
            .filter_map(|(names, file)| {
                let (name, aliases) = names.split_first()?;
                let label = if aliases.is_empty() {
                    name.clone()
                } else {
                    format!("{name} (also: {})", aliases.join(", "))
                };
                Some((label, path(file)))
            })
            .collect();
        entries.extend(self.ignored.iter().map(|(name, file)| {
            (
                format!("tmt-{name}"),
                format!("ignored: reserved core command ({})", path(file)),
            )
        }));
        if entries.is_empty() {
            return Vec::new();
        }
        vec![tmt_cli_style::HelpSection {
            title: "Extensions".into(),
            entries,
        }]
    }
}

/// Names whose commands resolve to the same file are one extension. The
/// longest name leads (ties alphabetical); names on different files stay apart.
fn group_aliases(
    extensions: Vec<(String, std::path::PathBuf, std::path::PathBuf)>,
) -> Vec<(Vec<String>, std::path::PathBuf)> {
    let mut groups: Vec<(Vec<String>, std::path::PathBuf, std::path::PathBuf)> = Vec::new();
    for (name, path, file) in extensions {
        match groups.iter_mut().find(|(_, _, existing)| *existing == file) {
            Some((names, _, _)) => names.push(name),
            None => groups.push((vec![name], path, file)),
        }
    }
    let mut grouped = groups
        .into_iter()
        .map(|(mut names, path, _)| {
            names.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
            (names, path)
        })
        .collect::<Vec<_>>();
    grouped.sort_by(|a, b| a.0[0].cmp(&b.0[0]));
    grouped
}

#[cfg(test)]
mod tests {
    use super::group_aliases;
    use std::path::PathBuf;

    #[test]
    fn names_resolving_to_one_file_are_grouped_and_others_stay_apart() {
        let entry = |name: &str, file: &str| {
            (
                name.to_owned(),
                PathBuf::from(format!("/bin/tmt-{name}")),
                PathBuf::from(file),
            )
        };
        let grouped = group_aliases(vec![
            entry("office", "/lib/office"),
            entry("sq", "/lib/squad"),
            entry("squad", "/lib/squad"),
            entry("sqx", "/lib/other"),
        ]);
        assert_eq!(
            grouped,
            [
                (vec!["office".to_owned()], PathBuf::from("/bin/tmt-office")),
                (
                    vec!["squad".to_owned(), "sq".to_owned()],
                    PathBuf::from("/bin/tmt-sq")
                ),
                (vec!["sqx".to_owned()], PathBuf::from("/bin/tmt-sqx")),
            ]
        );
    }
}
