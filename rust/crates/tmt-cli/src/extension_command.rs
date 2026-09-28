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
    for (name, path) in adapter::discover(&std::env::var_os("PATH").unwrap_or_default())? {
        if reserved.contains(&name) {
            writeln!(
                output,
                "Ignored tmt-{name}: reserved core command ({path:?})"
            )?;
        } else if valid_extension_name(&name) {
            writeln!(output, "Extension {name}: {path:?}")?;
        }
    }
    Ok(())
}
