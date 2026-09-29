//! Compose pure command grammar with the PATH execution adapter.

use crate::grammar::extensions;
use std::{
    ffi::OsString,
    io::{self, Write},
};
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
        writeln!(
            io::stderr(),
            "Could not execute extension '{}': {error}",
            executable.display()
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

pub fn write_discovered(output: &mut impl Write) -> io::Result<()> {
    let reserved = extensions::reserved(&crate::grammar::grammar());
    let mut extensions = Vec::new();
    for (name, path) in adapter::discover(&std::env::var_os("PATH").unwrap_or_default())? {
        if reserved.contains(&name) {
            writeln!(
                output,
                "Ignored tmt-{name}: reserved core command ({path:?})"
            )?;
        } else if valid_extension_name(&name) {
            let file = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            extensions.push((name, path, file));
        }
    }
    for (names, path) in group_aliases(extensions) {
        match names.split_first() {
            Some((name, [])) => writeln!(output, "Extension {name}: {path:?}")?,
            Some((name, aliases)) => writeln!(
                output,
                "Extension {name} (also: {}): {path:?}",
                aliases.join(", ")
            )?,
            None => {}
        }
    }
    Ok(())
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
