//! Storage-backed completion composition; grammar owns pure context parsing.

use crate::grammar::completion::{Context, context};
use std::{
    ffi::OsString,
    io::{self, Write},
};
use tmt_adapters::{config::ConfigPaths, storage::Storage};

/// Internal line protocol: a context tag, followed by names or a numeric command
/// offset. Names cannot contain line breaks. Lookup failure is deliberately
/// silent; completion must not create storage or disrupt the user's prompt.
pub fn query(words: &[OsString], output: &mut impl Write) -> io::Result<()> {
    if extension_candidates(words, output)? {
        return Ok(());
    }
    match context(words) {
        Context::Static => writeln!(output, "static"),
        Context::Command { offset } => writeln!(output, "command\n{offset}"),
        Context::Identities { prefix, remembered } => {
            writeln!(output, "identities")?;
            if let Some(candidates) = ConfigPaths::discover().ok().and_then(|paths| {
                Storage::identity_candidates(&paths.database, &prefix, remembered).ok()
            }) {
                for identity in candidates {
                    writeln!(output, "{}", identity.name)?;
                }
            }
            Ok(())
        }
    }
}

/// Only root discovery enumerates PATH. A selected extension uses exact lookup;
/// all core operands still belong to the grammar and never probe extensions.
fn extension_candidates(words: &[OsString], output: &mut impl Write) -> io::Result<bool> {
    use tmt_adapters::extension_command as adapter;
    use tmt_core::extension_command::valid_extension_name;
    let reserved = crate::grammar::extensions::reserved(&crate::grammar::grammar());
    let root = words.len() == 1 || words.len() == 2 && words[0] == "help";
    let search = std::env::var_os("PATH").unwrap_or_default();
    if root {
        let prefix = words.last().and_then(|word| word.to_str()).unwrap_or("");
        writeln!(output, "root")?;
        for name in adapter::discover(&search)?.keys() {
            if valid_extension_name(name) && !reserved.contains(name) && name.starts_with(prefix) {
                writeln!(output, "{name}")?;
            }
        }
        return Ok(true);
    }
    let Some(name) = words.first().and_then(|word| word.to_str()) else {
        return Ok(false);
    };
    if words.len() < 2 || reserved.contains(name) || !valid_extension_name(name) {
        return Ok(false);
    }
    let candidates = adapter::resolve(name, &search)
        .ok()
        .flatten()
        .and_then(|path| {
            std::env::current_exe()
                .ok()
                .and_then(|tmt| adapter::complete(&path, &words[1..], &tmt))
        });
    if let Some(candidates) = candidates {
        writeln!(output, "candidates")?;
        for candidate in candidates {
            writeln!(output, "{candidate}")?;
        }
    } else {
        writeln!(output, "files")?;
    }
    Ok(true)
}
