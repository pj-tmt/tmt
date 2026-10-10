//! Hidden `tmt remote` subcommands, each with why. Remote has none: every command is a
//! user action shown in help (`design/cli-style.md`, "Hidden commands"). Only a protocol
//! entry that a shell or host invokes belongs here, and an entry needs a reason.
//! The hidden `serve --worker` option is not a subcommand and is outside this guard.

pub const HIDDEN: &[(&str, &str)] = &[];
