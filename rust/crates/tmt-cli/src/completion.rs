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
