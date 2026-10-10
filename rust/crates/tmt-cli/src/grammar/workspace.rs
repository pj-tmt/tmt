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
    .subcommand(storage(spec!("restore", "Restore layout, remembered agents and TMT commands", details = "Existing sessions are skipped whole. Agents use tmt resume; missing or stale sessions need you. Recorded TMT commands restart with their arguments. Failures retain partial creations. Use workspace show to preview.", [
        "Restore the workspace after its server is gone" => "tmt workspace restore --socket /tmp/workspace.sock",
        "Restore only the layout" => "tmt workspace restore --layout-only --socket /tmp/workspace.sock",
    ])).arg(socket()).arg(Arg::new("layout-only").long("layout-only").action(ArgAction::SetTrue).help("Create layout and shells without reviving identities or recorded commands")))
}

fn socket() -> Arg {
    Arg::new("socket")
        .long("socket")
        .value_name("PATH")
        .help("Exact original tmux socket path; no default-server or directory search")
}
