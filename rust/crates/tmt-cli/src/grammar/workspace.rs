//! Read-only workspace recovery preview.

use crate::grammar::{base, storage};
use clap::{Arg, Command};

pub(in crate::grammar) fn workspace() -> Command {
    base(spec!("workspace", "Inspect remembered workspace recovery", [
        "Preview a remembered workspace" => "tmt workspace show",
    ])).subcommand(storage(spec!("show", "Preview layout and pane recovery without changing state", details = "Existing sessions are skipped. Unknown live state grants no restore authority; each launch must recheck its identity and remembered session. Select the original socket explicitly when its pane is gone.", [
        "Preview this server's snapshot" => "tmt workspace show",
        "Select a snapshot after its server is gone" => "tmt workspace show --socket /tmp/workspace.sock --json",
    ])).arg(Arg::new("socket").long("socket").value_name("PATH").help("Exact original tmux socket path; no default-server or directory search")))
}
