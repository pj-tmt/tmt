//! Squad commands that do not yet follow the help style, with the rules each
//! still breaks. The list only shrinks and is empty: every command has a
//! summary and examples, and `tmt ops squad help <command>` prints what
//! `<command> -h` prints. A new command that breaks a rule fails the walk
//! instead of being listed here.

use tmt_cli_style::audit::Rule;

pub const MIGRATING: &[(&str, &[Rule])] = &[];

/// Hidden Squad subcommands, each with why. Board-only actions are not
/// commands (`design/cli-style.md`, "Hidden commands"); only a protocol entry
/// that a shell or host invokes belongs here.
pub const HIDDEN: &[(&str, &str)] = &[(
    "tmt ops __complete",
    "shell completion scripts ask it for candidates",
)];
