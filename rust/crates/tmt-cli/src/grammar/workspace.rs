//! Workspace recovery preview and explicitly requested layout creation.

use crate::grammar::{base, storage};
use clap::{Arg, ArgAction, Command};

pub(in crate::grammar) fn workspace() -> Command {
    base(spec!("workspace", "Inspect and restore remembered workspace recovery", [
        "Preview a remembered workspace" => "tmt workspace show",
    ])).subcommand(storage(spec!("show", "Preview layout and pane recovery without changing state", details = "Existing sessions are skipped. Unknown live state grants no restore authority; each launch must recheck its identity and remembered session. Select the original socket explicitly when its pane is gone.", [
        "Preview this server's snapshot" => "tmt workspace show",
        "Select a snapshot after its server is gone" => "tmt workspace show --socket /tmp/workspace.sock --json",
    ])).arg(socket()))
    .subcommand(storage(spec!("restore", "Restore missing sessions as ordinary shells", details = "Creates layout only, including after reboot. Existing sessions are skipped whole. Failures retain partial creations. Only an invocation-owned unused bootstrap shell may be removed after linking and fresh verification.", [
        "Restore layout after its server is gone" => "tmt workspace restore --layout-only --socket /tmp/workspace.sock",
    ])).arg(socket()).arg(Arg::new("layout-only").long("layout-only").required(true).action(ArgAction::SetTrue).help("Create layout and shells without reviving identities or recorded commands")))
}

fn socket() -> Arg {
    Arg::new("socket")
        .long("socket")
        .value_name("PATH")
        .help("Exact original tmux socket path; no default-server or directory search")
}
